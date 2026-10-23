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

use super::column_text::ColumnText;
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
    /// What the grid was drawn with, the pieces of each line joined ([`joined_edges`]): its
    /// rules, the edges of its filled areas (not `ruled`), and each open side as a rule along
    /// it -- where a table drawn without a frame ends, its rules' ends bound it as a frame
    /// would. They tell which cell edges are drawn: a cell with nothing drawn along an edge its
    /// neighbours have is merged across it (see [`merged_cells`]). Empty for a grid whose
    /// boundaries were not all drawn (see `ruled_rows`): it has no merged cells.
    pub(crate) rules: Vec<AxisSegment>,
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
    let open_rows = open_sides(component, Axis::Vertical, &row_positions, config);
    let open_columns = open_sides(component, Axis::Horizontal, &col_positions, config);
    // Each axis needs two boundaries the rules draw: its own rules, and where the rules across it
    // end -- an open side, or a filled edge there (a shaded header band as wide as the rules). A
    // table of two columns ruled only between them -- one column rule, the rules between its rows
    // running past it to the table's sides -- has one rule of its own and two such ends. A bar's
    // edge is no end: a chart's gridlines run on past its bars. One rule each way is a cross, not
    // a table: one axis must be ruled twice.
    let ruled_ends = |positions: &[f32], own: &[f32], open: &[f32], across: Axis| {
        let ends = joined_edges(component, across, true, config.cluster_tolerance);
        let near = |a: f32, b: f32| (a - b).abs() <= config.cluster_tolerance;
        own.len()
            + open.len()
            + positions
                .iter()
                .filter(|&&p| !own.iter().chain(open).any(|&q| near(p, q)))
                .filter(|&&p| ends.iter().any(|r| near(r.lo, p) || near(r.hi, p)))
                .count()
    };
    // `y_rules` are the horizontal rules, between rows; `x_rules` the vertical ones.
    if ruled_ends(&row_positions, &y_rules, &open_rows, Axis::Vertical) < 2
        || ruled_ends(&col_positions, &x_rules, &open_columns, Axis::Horizontal) < 2
        || y_rules.len().max(x_rules.len()) < 2
    {
        return None;
    }
    row_positions.extend(open_rows.iter().copied());
    col_positions.extend(open_columns.iter().copied());
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

    let (top_y, bottom_y) = (*row_positions.first()?, *row_positions.last()?);
    let (left_x, right_x) = (*col_positions.first()?, *col_positions.last()?);
    let mut rules = Vec::new();
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for ruled in [true, false] {
            rules.extend(joined_edges(
                component,
                axis,
                ruled,
                config.cluster_tolerance,
            ));
        }
    }
    let side = |axis, pos, lo, hi| AxisSegment {
        axis,
        pos,
        lo,
        hi,
        ruled: true,
    };
    rules.extend(
        open_rows
            .iter()
            .map(|&y| side(Axis::Horizontal, y, left_x, right_x)),
    );
    rules.extend(
        open_columns
            .iter()
            .map(|&x| side(Axis::Vertical, x, bottom_y, top_y)),
    );

    Some(LatticeGrid {
        top_y,
        bottom_y,
        left_x,
        right_x,
        row_bounds: row_positions,
        col_bounds: col_positions,
        rules,
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
/// An axis needs a ruled boundary (and, with its open sides, two -- see `grid_from_component`).
/// Filled areas complete a ruled grid, but boundaries from filled areas alone are a figure — the
/// bars of a bar chart between its stroked gridlines.
///
/// And a filled edge is a boundary only where it spans a cell of the other axis's rules
/// (`across`): one unbroken stretch (pieces at most [`FILL_GAP`] apart) that crosses one, or
/// meets one at each end. A shaded row does, cell by cell; a highlight drawn behind one line
/// of a cell's text stops short of the rules around the cell, and taking each such edge as a
/// row boundary split a three-line header cell into three rows. A bar of a chart set in a
/// table's cells starts at its cell's rule and ends at its value, and as a row boundary split
/// every row of the table in two.
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
    if positions.is_empty() {
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
        // A stretch crosses a rule, or meets one at each end: a bar set in a cell starts at the
        // cell's rule and stops short of the next, and bounds no row.
        let meets = |end: f32| across.iter().any(|&r| (r - end).abs() <= FILL_TOUCH);
        axis == Axis::Vertical
            || stretches.iter().any(|&(lo, hi)| {
                across
                    .iter()
                    .any(|&r| r > lo + FILL_TOUCH && r < hi - FILL_TOUCH)
                    || (meets(lo) && meets(hi))
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
/// axis whose rules run across the `positions` — run on past the outermost of those positions,
/// the grid's last row or column lies between that position and the rules' ends.
///
/// Rules set only between the cells of a table (booktabs-style rules under the header and
/// between rows, column rules between columns) leave its outer rows and columns open: the
/// first column has no rule on its left, the last row none below it. The rules across still
/// span the whole table, so their ends bound it. A side is open when the rules at both ends of
/// the table overhang it by at least [`OPEN_SIDE_MIN`], and every other rule across either
/// overhangs it too or stops at one of the `positions` — the edge of a cell that spans rows in
/// the open column (a row label over three rows has no rules between them in its own column).
/// The boundary is the nearest end of the overhanging rules, so every one of them reaches it.
/// A framed table's rules stop at the frame and none overhang.
///
/// A rule is what is drawn along one line, however many pieces it is drawn in: a table whose
/// rules are drawn cell by cell has, under its header, a piece starting at each column rule —
/// counted as rules of their own, none of those but the first overhangs, and the table lost
/// both its outer columns.
fn open_sides(
    component: &[AxisSegment],
    across: Axis,
    positions: &[f32],
    config: &LatticeConfig,
) -> Vec<f32> {
    let rules = joined_edges(component, across, true, config.cluster_tolerance);
    let (Some(first), Some(last)) = (
        positions.iter().copied().reduce(f32::min),
        positions.iter().copied().reduce(f32::max),
    ) else {
        return Vec::new();
    };
    let (Some(outer_low), Some(outer_high)) = (
        rules.iter().map(|r| r.pos).reduce(f32::min),
        rules.iter().map(|r| r.pos).reduce(f32::max),
    ) else {
        return Vec::new();
    };
    let at_a_boundary = |end: f32| {
        positions
            .iter()
            .any(|&p| (p - end).abs() <= config.cluster_tolerance)
    };
    let mut sides = Vec::new();
    for low in [true, false] {
        let overhangs = |r: &AxisSegment| {
            if low {
                r.lo <= first - OPEN_SIDE_MIN
            } else {
                r.hi >= last + OPEN_SIDE_MIN
            }
        };
        let end = |r: &AxisSegment| if low { r.lo } else { r.hi };
        let outer_overhangs = |pos: f32| {
            rules
                .iter()
                .any(|r| (r.pos - pos).abs() <= config.cluster_tolerance && overhangs(r))
        };
        if outer_overhangs(outer_low)
            && outer_overhangs(outer_high)
            && rules.iter().all(|r| overhangs(r) || at_a_boundary(end(r)))
        {
            // The nearest end, so every overhanging rule reaches it.
            let ends = rules.iter().filter(|r| overhangs(r)).map(end);
            sides.extend(if low {
                ends.reduce(f32::max)
            } else {
                ends.reduce(f32::min)
            });
        }
    }
    sides
}

/// The segments of `axis` that are rules (`ruled`) or filled areas' edges (not), with the
/// pieces of each line joined: collinear pieces that overlap or abut (within `tolerance`) are
/// one rule.
fn joined_edges(
    component: &[AxisSegment],
    axis: Axis,
    ruled: bool,
    tolerance: f32,
) -> Vec<AxisSegment> {
    let mut pieces: Vec<AxisSegment> = component
        .iter()
        .filter(|s| s.axis == axis && s.ruled == ruled)
        .copied()
        .collect();
    pieces.sort_by(|a, b| a.pos.total_cmp(&b.pos).then(a.lo.total_cmp(&b.lo)));
    let mut rules: Vec<AxisSegment> = Vec::new();
    for piece in pieces {
        match rules.iter_mut().find(|r| r.touches(&piece, tolerance)) {
            Some(rule) => {
                rule.lo = rule.lo.min(piece.lo);
                rule.hi = rule.hi.max(piece.hi);
            }
            None => rules.push(piece),
        }
    }
    rules
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

    // Each line of text is in one row: the row the middle of its main text's letters is in --
    // its largest, so a subscript lowered across a row boundary, or to within a couple of
    // points of the table's top rule, stays with the line it marks.
    let mut line_row: Vec<Option<usize>> = vec![None; spans.len()];
    for line in group_into_rows(spans, CELL_LINE_TOLERANCE) {
        let row = line
            .spans
            .iter()
            .max_by(|a, b| a.font_size.total_cmp(&b.font_size))
            .and_then(|main| bin_index(&grid.row_bounds, letters_middle(main)));
        for &i in &line.sources {
            line_row[i] = row;
        }
    }

    // Each span by the column it starts in (and `column_holding`'s nudge) first, so every
    // column's text is known; then each span by the column whose text it lies over (see
    // `ColumnText::column_for`).
    let mut placed = Vec::new();
    let mut column_text = ColumnText::new(cols);
    for i in order {
        let span = &spans[i];
        let (Some(r), Some(start), Some(otherwise)) = (
            line_row[i],
            bin_index(&grid.col_bounds, span.x),
            column_holding(&grid.col_bounds, span),
        ) else {
            continue;
        };
        if !span.text.trim().is_empty() {
            let (left, right) = span.ink_extent();
            column_text.add(r, otherwise, left, right);
        }
        placed.push((i, r, start, otherwise));
    }
    for (i, r, start, otherwise) in placed {
        let span = &spans[i];
        let edge = grid.col_bounds.get(start + 1).copied();
        let (left, right) = span.ink_extent();
        let c = column_text.column_for(r, start, edge, left, right, otherwise);
        if !span.text.trim().is_empty() {
            cell_spans[r][c].push(span.clone());
        }
        consumed.push(i);
    }

    // A merged cell is read as one: the text of the positions it covers together, line by
    // line, so a label set across the rule its neighbours have reads in one piece.
    let mut cells: Vec<(Merge, Vec<String>)> = Vec::new();
    for merge in merged_cells(grid, &cell_spans) {
        let spans: Vec<TextSpan> = (merge.row..merge.row + merge.rows)
            .flat_map(|r| (merge.col..merge.col + merge.cols).map(move |c| (r, c)))
            .flat_map(|(r, c)| cell_spans[r][c].iter().cloned())
            .collect();
        // A cell reads line by line, a superscript with the line it marks (`0.31*`,
        // not `* 0.31`) -- the same rows stream-mode detection builds.
        let lines: Vec<String> = group_into_rows(&spans, CELL_LINE_TOLERANCE)
            .into_iter()
            .map(|line| TextLine::from_spans(line.spans).text().trim().to_string())
            .filter(|text| !text.is_empty())
            .collect();
        cells.push((merge, lines));
    }

    let non_empty = cells.iter().filter(|(_, lines)| !lines.is_empty()).count();
    let occupancy = non_empty as f32 / cells.len() as f32;
    if occupancy < MIN_OCCUPANCY {
        return None;
    }
    // One column is a table only as a list of items: every cell one line. A column whose
    // cells hold paragraphs is text drawn in boxes, and reads as text.
    if cols == 1 && !cells.iter().all(|(_, lines)| lines.len() == 1) {
        return None;
    }

    let mut table = Table::new();
    table.header_rows = if rows > 1 { 1 } else { 0 };
    for r in 0..rows {
        let row_cells: Vec<TableCell> = cells
            .iter()
            .filter(|(merge, _)| merge.row == r)
            .map(|(merge, lines)| {
                let mut cell =
                    TableCell::text(super::layout::trim_leaders(&lines.join(" ")).to_string());
                cell.rowspan = u8::try_from(merge.rows).unwrap_or(u8::MAX);
                cell.colspan = u8::try_from(merge.cols).unwrap_or(u8::MAX);
                cell
            })
            .collect();
        let table_row = if r == 0 && table.header_rows > 0 {
            TableRow::header(row_cells)
        } else {
            TableRow::new(row_cells)
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

/// Whether the text of two positions side by side, `left` and `right` of the boundary at `x`,
/// can be one cell's: one side is empty, a piece of text runs across `x` -- more than a font
/// size into each side, since a label in a band without the rule may start a little before
/// its column -- or a line goes on from one side to the other within a font size (`구 분` set
/// across the boundary, a word to each side of it). Labels centred in their own columns stand
/// further apart than that.
fn reads_across(left: &[TextSpan], right: &[TextSpan], x: f32) -> bool {
    if left.is_empty() || right.is_empty() {
        return true;
    }
    let runs_over = left.iter().chain(right).any(|s| {
        let (start, end) = s.ink_extent();
        start < x - s.font_size && end > x + s.font_size
    });
    runs_over || goes_on(left, right)
}

/// Whether a line of `left` goes on into `right`: a piece of each on one baseline, the right one
/// starting within a font size of where the left one's ink ends -- one line of text, however
/// many column boundaries lie between them.
fn goes_on(left: &[TextSpan], right: &[TextSpan]) -> bool {
    left.iter().any(|a| {
        right.iter().any(|b| {
            let size = a.font_size.max(b.font_size);
            (a.y - b.y).abs() <= size * CELL_LINE_TOLERANCE
                && b.ink_extent().0 - a.ink_extent().1 <= size
        })
    })
}

/// A cell of a grid: the row and column it starts in and how many of each it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Merge {
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}

/// The cells of `grid`, merged where its rules say so, in reading order (top row first, left
/// to right).
///
/// Two neighbouring positions of the grid are one cell when the edge between them is not
/// drawn while the edges it runs between are: neither a rule nor a filled area's edge lies
/// along it (a banded table's rows are bounded by their shading), and rules cross both its
/// ends. The boundary is there -- other rows (or columns) are drawn at it -- so this cell runs
/// across it, like a header over two columns or a row label over three rows. An edge whose
/// ends are not drawn either -- a header row standing above the top rule, a grid whose
/// boundaries were not all drawn -- is no evidence of a merge.
///
/// The text decides too. Two positions side by side are one cell only when one of them is
/// empty or their text reads as one ([`reads_across`]) -- two labels each standing in its own
/// column of a shaded header band, with no rule between them, are two cells -- and positions
/// joined along a row make one cell only while they hold one piece of text: two labels at the
/// ends of a band with empty positions between them stay in their columns. A merge down a
/// column takes in the text of one position at most: a column of row labels drawn in one box,
/// while the columns beside it are ruled row by row, holds a label per row, each on its row's
/// line -- and a label set over two lines of a cell spanning rows sits on the same lines, so
/// the drawing cannot tell them apart.
///
/// A merge has to make a rectangle: positions joined in any other shape are left as cells of
/// their own, since a cell is a rectangle and the drawing is then something else.
fn merged_cells(grid: &LatticeGrid, cell_spans: &[Vec<Vec<TextSpan>>]) -> Vec<Merge> {
    let rows = grid.row_count();
    let cols = grid.column_count();
    let tolerance = LatticeConfig::default().cluster_tolerance;
    // Whether a rule or a filled edge of `axis` at `pos` runs along `[lo, hi]` -- more than
    // touching it.
    let drawn_along = |axis: Axis, pos: f32, lo: f32, hi: f32| {
        grid.rules.iter().any(|r| {
            r.axis == axis
                && (r.pos - pos).abs() <= tolerance
                && r.hi.min(hi) - r.lo.max(lo) > tolerance
        })
    };
    // Whether a rule of `axis` at `pos` reaches the point `at` along it.
    let crossed_at = |axis: Axis, pos: f32, at: f32| {
        grid.rules.iter().any(|r| {
            r.ruled
                && r.axis == axis
                && (r.pos - pos).abs() <= tolerance
                && at >= r.lo - tolerance
                && at <= r.hi + tolerance
        })
    };
    // Whether anything of `axis` is drawn at `pos` anywhere: a boundary found from the text
    // (see [`with_unruled_rows`]) is drawn nowhere, and no evidence of a merge.
    let drawn_at = |axis: Axis, pos: f32| {
        grid.rules
            .iter()
            .any(|r| r.axis == axis && (r.pos - pos).abs() <= tolerance)
    };
    let joins_right = |r: usize, c: usize| {
        let x = grid.col_bounds[c + 1];
        let (y_top, y_bottom) = (grid.row_bounds[r], grid.row_bounds[r + 1]);
        drawn_at(Axis::Vertical, x)
            && !drawn_along(Axis::Vertical, x, y_bottom, y_top)
            && crossed_at(Axis::Horizontal, y_top, x)
            && crossed_at(Axis::Horizontal, y_bottom, x)
            && reads_across(&cell_spans[r][c], &cell_spans[r][c + 1], x)
    };
    let joins_below = |r: usize, c: usize| {
        let y = grid.row_bounds[r + 1];
        let (x_left, x_right) = (grid.col_bounds[c], grid.col_bounds[c + 1]);
        drawn_at(Axis::Horizontal, y)
            && !drawn_along(Axis::Horizontal, y, x_left, x_right)
            && crossed_at(Axis::Vertical, x_left, y)
            && crossed_at(Axis::Vertical, x_right, y)
    };
    let has_text = |i: usize| !cell_spans[i / cols][i % cols].is_empty();
    // Joins along a row are decided per run of positions with no column rule between them: a
    // run holding one piece of text is one cell, but a run holding several -- labels standing
    // apart in one band, `bit 7` at its start and `bit 0` at its end -- keeps each label in
    // the column it is drawn in. Its empty positions join neither label: joined one by one,
    // they would carry one label into the other. Text that goes on from one position to a
    // later one ([`goes_on`]) is one piece, whatever empty positions its line runs over.
    let mut joins_along_row = vec![false; rows * cols];
    for r in 0..rows {
        let text = |c: usize| has_text(r * cols + c);
        let mut start = 0;
        while start < cols {
            let mut end = start;
            while end + 1 < cols && joins_right(r, end) {
                end += 1;
            }
            let mut pieces = 0;
            let mut last: Option<usize> = None;
            for c in (start..=end).filter(|&c| text(c)) {
                if !last.is_some_and(|l| goes_on(&cell_spans[r][l], &cell_spans[r][c])) {
                    pieces += 1;
                }
                last = Some(c);
            }
            for c in start..end {
                joins_along_row[r * cols + c] = pieces <= 1 || (text(c) && text(c + 1));
            }
            start = end + 1;
        }
    }

    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        parent[i] = root;
        root
    }
    let mut parent: Vec<usize> = (0..rows * cols).collect();
    // Whether each position's merge so far holds text: a merge down a column takes in at most
    // one piece of it, so an empty position between two labels joins one of them, not both.
    let mut texted: Vec<bool> = (0..rows * cols).map(has_text).collect();
    let mut union = |a: usize, b: usize, one_text: bool| {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb && !(one_text && texted[ra] && texted[rb]) {
            let (keep, gone) = (ra.min(rb), ra.max(rb));
            parent[gone] = keep;
            texted[keep] |= texted[gone];
        }
    };
    for r in 0..rows {
        for c in 0..cols {
            if c + 1 < cols && joins_along_row[r * cols + c] {
                union(r * cols + c, r * cols + c + 1, false);
            }
            if r + 1 < rows && joins_below(r, c) {
                union(r * cols + c, (r + 1) * cols + c, true);
            }
        }
    }

    let mut members: std::collections::BTreeMap<usize, Vec<(usize, usize)>> =
        std::collections::BTreeMap::new();
    for i in 0..rows * cols {
        members
            .entry(find(&mut parent, i))
            .or_default()
            .push((i / cols, i % cols));
    }
    let mut cells: Vec<Merge> = Vec::new();
    for positions in members.into_values() {
        let row = positions.iter().map(|p| p.0).min().unwrap_or(0);
        let col = positions.iter().map(|p| p.1).min().unwrap_or(0);
        let rows = positions.iter().map(|p| p.0).max().unwrap_or(0) + 1 - row;
        let cols = positions.iter().map(|p| p.1).max().unwrap_or(0) + 1 - col;
        if positions.len() == rows * cols {
            cells.push(Merge {
                row,
                col,
                rows,
                cols,
            });
        } else {
            cells.extend(positions.into_iter().map(|(row, col)| Merge {
                row,
                col,
                rows: 1,
                cols: 1,
            }));
        }
    }
    cells.sort_by_key(|m| (m.row, m.col));
    cells
}

/// The column (index into the columns `col_bounds` bound) that holds `span` when the text of
/// the other rows gives no answer (see `ColumnText::column_for`): the one it starts in — unless it starts just short of that column's right boundary, within a font size of it,
/// and more of it lies past the boundary than before it. A row without a rule between two
/// columns (a header band shaded across both) lets a cell's text start a few points before the
/// boundary the rows below are ruled at. A span starting further into its column stays there
/// however far it runs on: grids ruled finer than their columns, and estimated widths, run
/// text past boundaries it was never meant to cross.
fn column_holding(col_bounds: &[f32], span: &TextSpan) -> Option<usize> {
    let c = bin_index(col_bounds, span.x)?;
    let (Some(&edge), Some(&next_end)) = (col_bounds.get(c + 1), col_bounds.get(c + 2)) else {
        return Some(c);
    };
    let before = edge - span.x;
    let past = span.ink_extent().1.min(next_end) - edge;
    Some(if before <= span.font_size && past > before {
        c + 1
    } else {
        c
    })
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
        .filter(|s| {
            letters_middle(s) > grid.top_y + CELL_BOUNDARY_TOLERANCE && !s.text.trim().is_empty()
        })
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

/// `grid` with each body row that holds several records divided into the rows its text sets.
///
/// A table ruled above, under its header and below -- and down its columns, or not -- but not
/// between its body rows has one body row in its grid, and read as one it puts every value of
/// a column into one cell. Its text has the rows: each line of the body has a value in every
/// column, side by side on one baseline. A row under the first is divided into its lines when
/// it has [`UNRULED_ROW_MIN_LINES`] lines or more, text in [`UNRULED_ROW_MIN_COLUMNS`] columns
/// or more, and a line of text in every one of them on most of its lines
/// ([`UNRULED_ROW_MIN_FILL`]). A row of wrapped cells -- one column running on for several
/// lines beside columns of one or two -- is one row, its cells read as one.
pub(crate) fn with_unruled_rows(grid: &LatticeGrid, spans: &[TextSpan]) -> LatticeGrid {
    if grid.column_count() < UNRULED_ROW_MIN_COLUMNS {
        return grid.clone();
    }
    let text: Vec<&TextSpan> = spans
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .filter(|s| bin_index(&grid.col_bounds, s.x).is_some())
        .collect();
    let mut row_bounds = vec![grid.row_bounds[0]];
    for r in 0..grid.row_count() {
        let (top, bottom) = (grid.row_bounds[r], grid.row_bounds[r + 1]);
        if r > 0 {
            let body: Vec<TextSpan> = text
                .iter()
                .filter(|s| s.y <= top && s.y >= bottom)
                .map(|s| (*s).clone())
                .collect();
            let lines = group_into_rows(&body, CELL_LINE_TOLERANCE);
            if holds_records(grid, &lines) {
                row_bounds.extend(lines.windows(2).map(|w| (w[0].y + w[1].y) / 2.0));
            }
        }
        row_bounds.push(bottom);
    }
    LatticeGrid {
        row_bounds,
        ..grid.clone()
    }
}

/// Whether the `lines` of one row of `grid` are records of their own (see
/// [`with_unruled_rows`]).
fn holds_records(grid: &LatticeGrid, lines: &[super::table_detector::TableRowData]) -> bool {
    if lines.len() < UNRULED_ROW_MIN_LINES {
        return false;
    }
    let lines_in = |c: usize| {
        lines
            .iter()
            .filter(|line| {
                line.spans
                    .iter()
                    .any(|s| bin_index(&grid.col_bounds, s.x) == Some(c))
            })
            .count()
    };
    let filled: Vec<usize> = (0..grid.column_count())
        .map(lines_in)
        .filter(|&n| n > 0)
        .collect();
    if filled.len() < UNRULED_ROW_MIN_COLUMNS
        || !filled
            .iter()
            .all(|&n| n as f32 >= lines.len() as f32 * UNRULED_ROW_MIN_FILL)
    {
        return false;
    }
    // Columns of running text -- an article set in columns inside a frame -- fill every column
    // on every line too, one line spacing for all of them. Their lines are prose, each column
    // read down, not entries: the row is records only when its lines, column by column, are not.
    let column_lines: Vec<super::table_detector::TableRowData> = lines
        .iter()
        .flat_map(|line| {
            (0..grid.column_count()).filter_map(move |c| {
                let spans: Vec<TextSpan> = line
                    .spans
                    .iter()
                    .filter(|s| bin_index(&grid.col_bounds, s.x) == Some(c))
                    .cloned()
                    .collect();
                (!spans.is_empty()).then(|| super::table_detector::TableRowData {
                    y: line.y,
                    spans,
                    sources: Vec::new(),
                })
            })
        })
        .collect();
    !super::ruled_rows::reads_as_prose(&column_lines.iter().collect::<Vec<_>>())
}

/// The fewest lines a row needs to be divided into records: a cell of two lines -- a date over
/// its kind, a count over its share -- beside cells of two is one cell.
const UNRULED_ROW_MIN_LINES: usize = 3;

/// The share of a row's lines every column holding text in it must have a line on for the
/// lines to be records (see [`with_unruled_rows`]): most of them. Cells of two lines -- a name
/// over its hours -- beside one that holds two such pairs fill half the row's lines.
const UNRULED_ROW_MIN_FILL: f32 = 2.0 / 3.0;

/// The fewest columns holding text a body needs for its lines to be its rows: two lists side by
/// side in one ruled row (reagents beside equipment) share a line spacing without their lines
/// being records, and read as two cells of lists. A label beside two values or more is a
/// record.
const UNRULED_ROW_MIN_COLUMNS: usize = 3;

/// Boundary-tolerance for assigning a span to a grid cell (points). Grid
/// boundaries are exact ruling-line positions; a span whose baseline sits a
/// couple of points outside the nominal box (font metrics, hairline rounding)
/// should still land in its cell rather than being silently dropped.
const CELL_BOUNDARY_TOLERANCE: f32 = 2.0;

/// The height of the middle of a span's letters, between its descender and its ascender: what
/// decides the row a line of text is in (see [`build_table`]). Its baseline does not -- a caption
/// standing just above a table's top rule has its letters above the rule.
fn letters_middle(span: &TextSpan) -> f32 {
    (span.top() + span.bottom()) / 2.0
}

/// Find which `[boundaries[i], boundaries[i+1])` bin `value` falls into.
/// Works for either ascending (`col_bounds`) or descending (`row_bounds`)
/// boundary lists.
fn bin_index(boundaries: &[f32], value: f32) -> Option<usize> {
    let bins = || {
        boundaries.windows(2).enumerate().map(|(i, pair)| {
            let (lo, hi) = (pair[0].min(pair[1]), pair[0].max(pair[1]));
            (i, lo, hi)
        })
    };
    // The bin the value lies in; only a value outside every bin is given the tolerance -- a
    // value just inside one bin is not the bin before it's, however close to it.
    bins()
        .find(|&(_, lo, hi)| value >= lo && value <= hi)
        .or_else(|| {
            bins().find(|&(_, lo, hi)| {
                value >= lo - CELL_BOUNDARY_TOLERANCE && value <= hi + CELL_BOUNDARY_TOLERANCE
            })
        })
        .map(|(i, _, _)| i)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Horizontal,
    Vertical,
}

/// An axis-aligned ruling line: its position on the perpendicular axis and
/// its extent `[lo, hi]` along its own axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AxisSegment {
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
        // The shading bounds its rows as a rule would: no row is merged into the next.
        let spans = vec![
            span("a", 60.0, 287.0),
            span("b", 60.0, 267.0),
            span("c", 60.0, 247.0),
            span("1", 160.0, 287.0),
        ];
        let (table, _) = build_table(&grids[0], &spans).unwrap();
        assert!(!table.has_merged_cells());
        assert_eq!(table.rows.len(), 3);
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
            ink_width: None,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
            is_struck: false,
            direction: TextSpan::LEFT_TO_RIGHT,
            letter_spacing: 0.0,
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
            rules: Vec::new(),
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
            ink_width: None,
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

    /// A value inside a bin is in that bin, even within the tolerance of the bin before it:
    /// the tolerance only catches values just outside every bin.
    #[test]
    fn bin_index_prefers_the_bin_a_value_lies_in() {
        let descending = [30.0, 20.0, 10.0];
        assert_eq!(bin_index(&descending, 19.0), Some(1));
        assert_eq!(bin_index(&descending, 31.0), Some(0));
        assert_eq!(bin_index(&descending, 9.0), Some(1));
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

    /// Two columns ruled only between them: one column rule, the rules between the rows running
    /// past it on both sides. The rule and the rules' ends bound the two columns.
    #[test]
    fn two_columns_ruled_only_between_them_are_a_grid() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(240.0, 20.0, 420.0),
            h(200.0, 20.0, 420.0),
            v(120.0, 200.0, 300.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].col_bounds, vec![20.0, 120.0, 420.0]);
        assert_eq!(grids[0].row_count(), 3);
    }

    /// One rule each way, crossing -- a column rule and a rule across it -- is no table.
    #[test]
    fn one_rule_each_way_is_no_grid() {
        let lines = vec![h(250.0, 20.0, 420.0), v(120.0, 100.0, 400.0)];
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    /// A column rule the rules between the rows do not run past on one side leaves one column:
    /// the rule alone does not make two.
    #[test]
    fn one_column_rule_at_the_rules_end_is_no_second_column() {
        let lines = vec![
            h(300.0, 20.0, 120.0),
            h(250.0, 20.0, 120.0),
            h(200.0, 20.0, 120.0),
            v(120.0, 200.0, 300.0),
        ];
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    /// The same open table with its rules drawn cell by cell: the rule under the header is a
    /// piece per column, starting at each column rule. It is one rule all the same, and the
    /// outer columns stay in the table.
    #[test]
    fn an_open_table_ruled_cell_by_cell_keeps_its_outer_columns() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 120.0),
            h(280.0, 120.0, 270.0),
            h(280.0, 270.0, 420.0),
            h(210.0, 20.0, 420.0),
            v(120.0, 210.0, 300.0),
            v(270.0, 210.0, 300.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].col_bounds, vec![20.0, 120.0, 270.0, 420.0]);
    }

    /// A row label spanning rows in the open first column: the rules between those rows start
    /// at the first column rule, not past it. They stop at a boundary of the grid, and the
    /// column stays open.
    #[test]
    fn a_label_spanning_rows_in_an_open_column_keeps_the_column() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(260.0, 120.0, 420.0),
            h(240.0, 120.0, 420.0),
            h(220.0, 20.0, 420.0),
            v(120.0, 220.0, 280.0),
            v(270.0, 220.0, 300.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].col_bounds, vec![20.0, 120.0, 270.0, 420.0]);
        assert_eq!(grids[0].row_count(), 4);
    }

    /// An open table whose header label sits over two columns (no column rule in the header
    /// row) and whose row label sits over two rows (no rule between them in its column, the
    /// label centred on the two): each is one merged cell, its text read in one piece.
    #[test]
    fn cells_without_a_rule_their_neighbours_have_are_merged() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(260.0, 120.0, 420.0),
            h(240.0, 20.0, 420.0),
            v(120.0, 240.0, 280.0),
            v(270.0, 240.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        let spans = vec![
            span("Item", 100.0, 287.0),
            span("kind", 128.0, 287.0),
            span("2020", 280.0, 287.0),
            span("GMV", 30.0, 257.0),
            span("A", 130.0, 267.0),
            span("1", 280.0, 267.0),
            span("B", 130.0, 247.0),
            span("2", 280.0, 247.0),
        ];
        let (table, consumed) = build_table(grid, &spans).unwrap();
        assert_eq!(consumed.len(), 8);
        let rows: Vec<Vec<(String, u8, u8)>> = table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| (c.plain_text(), c.rowspan, c.colspan))
                    .collect()
            })
            .collect();
        let cell = |t: &str, r: u8, c: u8| (t.to_string(), r, c);
        assert_eq!(
            rows,
            vec![
                vec![cell("Item kind", 1, 2), cell("2020", 1, 1)],
                vec![cell("GMV", 2, 1), cell("A", 1, 1), cell("1", 1, 1)],
                vec![cell("B", 1, 1), cell("2", 1, 1)],
            ]
        );
        assert_eq!(table.column_count(), 3);
    }

    /// Two labels in a header band with no rule between them, each centred in its own column,
    /// are two cells -- a line of text does not go on into the next column.
    #[test]
    fn two_labels_in_an_unruled_header_band_stay_two_cells() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(260.0, 20.0, 420.0),
            v(20.0, 260.0, 300.0),
            v(120.0, 260.0, 300.0),
            v(270.0, 260.0, 280.0),
            v(420.0, 260.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        // `Meiosis` starts a few points before its column, as a centred label in a band
        // without the rule may.
        let spans = vec![
            span("Mitosis", 170.0, 287.0),
            span("Meiosis", 266.0, 287.0),
            span("one", 130.0, 267.0),
            span("two", 280.0, 267.0),
        ];
        let (table, _) = build_table(grid, &spans).unwrap();
        assert!(!table.has_merged_cells());
        assert_eq!(table.rows[0].cells[1].plain_text(), "Mitosis");
        assert_eq!(table.rows[0].cells[2].plain_text(), "Meiosis");
    }

    /// A band drawn without column rules under a row ruled cell by cell, a label at each end
    /// (a register map's `bit 3` … `bit 0`): the labels stand apart, so the band is not one
    /// cell -- each label stays in the column it is drawn in, the positions between them empty.
    /// Empty positions joining their neighbours one by one must not carry one label into the
    /// other.
    #[test]
    fn labels_at_the_ends_of_an_unruled_band_stay_in_their_columns() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(260.0, 20.0, 420.0),
            v(20.0, 260.0, 300.0),
            v(120.0, 280.0, 300.0),
            v(220.0, 280.0, 300.0),
            v(320.0, 280.0, 300.0),
            v(420.0, 260.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        assert_eq!(grid.column_count(), 4);
        let spans = vec![
            span("RTCSEC", 30.0, 287.0),
            span("ST", 130.0, 287.0),
            span("SECTEN", 230.0, 287.0),
            span("SECONE", 330.0, 287.0),
            span("bit 3", 25.0, 267.0),
            span("bit 0", 390.0, 267.0),
        ];
        let (table, _) = build_table(grid, &spans).unwrap();
        let band: Vec<String> = table.rows[1].cells.iter().map(|c| c.plain_text()).collect();
        assert_eq!(band, ["bit 3", "", "", "bit 0"]);
        assert!(
            table.rows[1].cells.iter().all(|c| c.colspan == 1),
            "{band:?}"
        );
    }

    /// The same band holding one line of text that runs on over the column boundaries -- a
    /// name with its note mark set right after it, `M.D.` falling in one column and `1*` two
    /// columns on -- is one piece of text, and the band one cell.
    #[test]
    fn a_line_running_on_across_an_unruled_band_is_one_cell() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(260.0, 20.0, 420.0),
            v(20.0, 260.0, 300.0),
            v(120.0, 280.0, 300.0),
            v(220.0, 280.0, 300.0),
            v(320.0, 280.0, 300.0),
            v(420.0, 260.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        let mut name = span("Stafford Warren, M.D.", 25.0, 267.0);
        name.width = 205.0;
        let spans = vec![
            span("Name", 30.0, 287.0),
            span("A", 130.0, 287.0),
            span("B", 230.0, 287.0),
            span("C", 330.0, 287.0),
            name,
            span("1*", 231.0, 270.0),
        ];
        let (table, _) = build_table(grid, &spans).unwrap();
        let band: Vec<String> = table.rows[1].cells.iter().map(|c| c.plain_text()).collect();
        assert_eq!(band.len(), 1, "{band:?}");
        assert!(
            band[0].starts_with("Stafford Warren, M.D.") && band[0].ends_with("1*"),
            "{band:?}"
        );
    }

    /// A caption standing just above a table's top rule, its subscript lowered to within a
    /// couple of points of the rule (`f` with `PCLK1` under it): the subscript's letters are
    /// above the rule, so it stays with its caption -- not in the table's first cell, leaving
    /// a hole in the caption's line.
    #[test]
    fn a_subscript_just_above_the_top_rule_stays_out_of_the_table() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 20.0, 420.0),
            h(260.0, 20.0, 420.0),
            v(20.0, 260.0, 300.0),
            v(220.0, 260.0, 300.0),
            v(420.0, 260.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        let mut subscript = span("PCLK1", 150.0, 302.0);
        subscript.font_size = 8.0;
        let spans = vec![
            span("fSCL (kHz)", 30.0, 287.0),
            span("I2C_CCR value", 230.0, 287.0),
            span("400", 30.0, 267.0),
            span("0x8019", 230.0, 267.0),
            subscript,
        ];
        let (table, consumed) = build_table(grid, &spans).unwrap();
        assert_eq!(table.rows[0].cells[0].plain_text(), "fSCL (kHz)");
        assert!(
            !consumed.contains(&4),
            "the subscript was taken into the table"
        );
    }

    /// A header label set across the boundary of two rows that its column has no rule at, its
    /// subscript lowered below the boundary (`f` with `SCL` under it, then `(kHz)`): the line
    /// is in one row, so the label reads as one cell spanning both -- not `f (kHz)` over `SCL`.
    #[test]
    fn a_label_with_a_subscript_across_a_row_boundary_stays_one_cell() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 220.0, 420.0),
            h(260.0, 20.0, 420.0),
            h(240.0, 20.0, 420.0),
            v(20.0, 240.0, 300.0),
            v(220.0, 240.0, 300.0),
            v(420.0, 240.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        // The label's letters stand around the boundary at 280; its subscript sits lower.
        let mut f = span("f", 30.0, 277.5);
        f.font_size = 9.0;
        let mut sub = span("SCL", 36.0, 275.5);
        sub.font_size = 7.2;
        let mut khz = span("(kHz)", 60.0, 277.5);
        khz.font_size = 9.0;
        let spans = vec![
            f,
            sub,
            khz,
            span("I2C_CCR value", 230.0, 287.0),
            span("RP = 4.7 k", 230.0, 267.0),
            span("400", 30.0, 247.0),
            span("0x8019", 230.0, 247.0),
        ];
        let (table, _) = build_table(grid, &spans).unwrap();
        let label = &table.rows[0].cells[0];
        assert_eq!(label.rowspan, 2, "{:?}", label.plain_text());
        let text = label.plain_text();
        assert!(
            text.contains('f') && text.contains("SCL") && text.contains("(kHz)"),
            "{text}"
        );
    }

    /// A column of row labels drawn as one box beside columns ruled row by row: a label per
    /// row, each in its own row, not one cell.
    #[test]
    fn row_labels_in_one_box_beside_ruled_rows_stay_one_per_row() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 120.0, 420.0),
            h(260.0, 120.0, 420.0),
            h(240.0, 20.0, 420.0),
            v(20.0, 240.0, 300.0),
            v(120.0, 240.0, 300.0),
            v(270.0, 240.0, 300.0),
            v(420.0, 240.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        let spans = vec![
            span("Rate", 30.0, 287.0),
            span("Total", 30.0, 267.0),
            span("Men", 30.0, 247.0),
            span("1", 130.0, 287.0),
            span("2", 130.0, 267.0),
            span("3", 130.0, 247.0),
        ];
        let (table, _) = build_table(grid, &spans).unwrap();
        assert!(!table.has_merged_cells());
        assert_eq!(table.rows[2].cells[0].plain_text(), "Men");
    }

    /// A grid ruled above, under its header and below, and down its three columns.
    fn header_and_body_lines() -> Vec<GraphicsLine> {
        vec![
            h(300.0, 50.0, 350.0),
            h(280.0, 50.0, 350.0),
            h(240.0, 50.0, 350.0),
            v(50.0, 240.0, 300.0),
            v(150.0, 240.0, 300.0),
            v(250.0, 240.0, 300.0),
            v(350.0, 240.0, 300.0),
        ]
    }

    /// Ruled above, under its header and below, and down its columns, but not between its
    /// body rows: the body's lines are its rows.
    #[test]
    fn a_body_ruled_into_no_rows_is_divided_by_its_lines() {
        let grid = &infer_grids(&header_and_body_lines(), &LatticeConfig::default())[0];
        let mut spans = vec![
            span("Item", 60.0, 287.0),
            span("2011", 160.0, 287.0),
            span("2010", 260.0, 287.0),
        ];
        for (i, (item, now, then)) in [
            ("Short", "27", "26"),
            ("Adequate", "58", "67"),
            ("Surplus", "3", "6"),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 270.0 - i as f32 * 12.0;
            spans.push(span(item, 60.0, y));
            spans.push(span(now, 160.0, y));
            spans.push(span(then, 260.0, y));
        }
        let grid = with_unruled_rows(grid, &spans);
        assert_eq!(grid.row_count(), 4);
        let (table, _) = build_table(&grid, &spans).unwrap();
        assert!(!table.has_merged_cells());
        assert_eq!(table.rows[2].cells[0].plain_text(), "Adequate");
        assert_eq!(table.rows[2].cells[2].plain_text(), "67");
    }

    /// An article's columns of running text inside a frame fill every column on every line,
    /// one line spacing for all of them -- but each column's lines are prose, not entries: the
    /// body stays one row, each column read down as one cell.
    #[test]
    fn columns_of_prose_in_a_frame_are_not_divided_into_records() {
        let grid = &infer_grids(&header_and_body_lines(), &LatticeConfig::default())[0];
        let columns = [
            [
                "Lilienthal was relentless in his",
                "study of air movement and wind,",
                "and he recognized quickly what",
                "is common knowledge among us",
            ],
            [
                "flight until the year after that.",
                "Before he flew, he published his",
                "theories on how man could fly in",
                "a book which was widely read by",
            ],
            [
                "Modern hang glider enthusiasts will",
                "also recognize and appreciate what",
                "these experiments revealed, that he",
                "could control his direction of flight",
            ],
        ];
        let mut spans = Vec::new();
        for (c, lines) in columns.iter().enumerate() {
            for (i, text) in lines.iter().enumerate() {
                let mut line = span(text, 55.0 + c as f32 * 100.0, 270.0 - i as f32 * 9.0);
                line.font_size = 7.0;
                line.width = 92.0;
                spans.push(line);
            }
        }
        let divided = with_unruled_rows(grid, &spans);
        assert_eq!(divided.row_count(), grid.row_count());
    }

    /// A body row whose one cell wraps over several lines beside cells of one line is one row.
    #[test]
    fn a_body_row_of_wrapped_cells_stays_one_row() {
        let grid = &infer_grids(&header_and_body_lines(), &LatticeConfig::default())[0];
        let spans = vec![
            span("Item", 60.0, 287.0),
            span("Notes", 160.0, 287.0),
            span("Alpha", 60.0, 270.0),
            span("first line", 160.0, 270.0),
            span("second line", 160.0, 258.0),
            span("third line", 160.0, 246.0),
            span("1", 260.0, 270.0),
        ];
        assert_eq!(with_unruled_rows(grid, &spans), *grid);
    }

    /// Cells of two lines each -- a name over its hours -- with one cell holding two such pairs:
    /// one row of cells, not four rows.
    #[test]
    fn cells_of_two_lines_beside_a_cell_of_four_stay_one_row() {
        let grid = &infer_grids(&header_and_body_lines(), &LatticeConfig::default())[0];
        let spans = vec![
            span("Mon", 60.0, 287.0),
            span("Tue", 160.0, 287.0),
            span("Wed", 260.0, 287.0),
            span("White", 60.0, 274.0),
            span("9am", 60.0, 264.0),
            span("Smith", 160.0, 274.0),
            span("8am", 160.0, 264.0),
            span("Audino", 260.0, 274.0),
            span("8am", 260.0, 264.0),
            span("Nocciolino", 260.0, 254.0),
            span("10am", 260.0, 244.0),
        ];
        assert_eq!(with_unruled_rows(grid, &spans), *grid);
    }

    /// Two lists side by side in one ruled row share their line spacing without their lines
    /// being records: the row stays one row of two lists.
    #[test]
    fn two_lists_side_by_side_in_one_row_stay_one_row() {
        let grid = &infer_grids(&header_and_body_lines(), &LatticeConfig::default())[0];
        let mut spans = vec![
            span("Reagents", 60.0, 287.0),
            span("Supplies", 160.0, 287.0),
        ];
        for (i, (left, right)) in [("Buffer", "Rack"), ("Enzyme", "Tips"), ("Water", "Marker")]
            .into_iter()
            .enumerate()
        {
            let y = 270.0 - i as f32 * 12.0;
            spans.push(span(left, 60.0, y));
            spans.push(span(right, 160.0, y));
        }
        assert_eq!(with_unruled_rows(grid, &spans), *grid);
    }

    /// A box of row labels with a position left empty between two labels (a sub-header row of
    /// the columns beside it): the empty position joins one label, and the two labels stay two
    /// cells.
    #[test]
    fn an_empty_position_between_two_labels_joins_one_of_them() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(280.0, 120.0, 420.0),
            h(260.0, 120.0, 420.0),
            h(240.0, 120.0, 420.0),
            h(220.0, 20.0, 420.0),
            v(20.0, 220.0, 300.0),
            v(120.0, 220.0, 300.0),
            v(270.0, 220.0, 300.0),
            v(420.0, 220.0, 300.0),
        ];
        let grid = &infer_grids(&lines, &LatticeConfig::default())[0];
        let spans = vec![
            span("Label", 30.0, 287.0),
            span("Value", 130.0, 287.0),
            span("Rate", 30.0, 267.0),
            span("1", 130.0, 267.0),
            span("share", 130.0, 247.0),
            span("Duration", 30.0, 227.0),
            span("2", 130.0, 227.0),
        ];
        let (table, _) = build_table(grid, &spans).unwrap();
        let texts: Vec<String> = table
            .rows
            .iter()
            .flat_map(|row| row.cells.iter().map(|c| c.plain_text()))
            .collect();
        assert!(texts.contains(&"Rate".to_string()), "{texts:?}");
        assert!(texts.contains(&"Duration".to_string()), "{texts:?}");
    }

    /// A header row standing above the top rule has no column rules, but nothing is drawn at
    /// its top either: its labels stay in their own columns.
    #[test]
    fn a_header_row_above_the_rules_is_not_merged() {
        let grid = &infer_grids(&open_border_lines(), &LatticeConfig::default())[0];
        let spans = vec![
            span("Name", 130.0, 306.0),
            span("Value", 280.0, 306.0),
            span("Alpha", 30.0, 280.0),
            span("one", 130.0, 280.0),
            span("two", 280.0, 280.0),
        ];
        let grid = with_header_above(grid, &spans);
        let (table, _) = build_table(&grid, &spans).unwrap();
        assert_eq!(table.rows[0].cells.len(), 3);
        assert!(!table.has_merged_cells());
    }

    /// A rule that stops short of the others somewhere that is no boundary of the grid is no
    /// evidence of an open side: the side stays closed.
    #[test]
    fn a_rule_stopping_short_off_any_boundary_does_not_open_a_side() {
        let lines = vec![
            h(300.0, 20.0, 420.0),
            h(260.0, 112.0, 420.0),
            h(210.0, 20.0, 420.0),
            v(120.0, 210.0, 300.0),
            v(270.0, 210.0, 300.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].col_bounds, vec![120.0, 270.0, 420.0]);
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

    /// A label starting a few points before a column boundary, in a row without the rule
    /// there, belongs to the column most of it lies in.
    #[test]
    fn a_span_belongs_to_the_column_most_of_it_lies_in() {
        let bounds = [100.0, 270.0, 446.0, 612.0];
        let meiosis = TextSpan {
            width: 46.0,
            ..span("Meiosis", 442.0, 300.0)
        };
        assert_eq!(column_holding(&bounds, &meiosis), Some(2));
        let mitosis = TextSpan {
            width: 43.0,
            ..span("Mitosis", 278.0, 300.0)
        };
        assert_eq!(column_holding(&bounds, &mitosis), Some(1));
        // Starting well inside its column, a span stays there however far it runs on.
        let long = TextSpan {
            width: 120.0,
            ..span("A long label", 410.0, 300.0)
        };
        assert_eq!(column_holding(&bounds, &long), Some(1));
        // Trailing spaces are advance, not ink: a value padded to a fixed width that ends
        // before the boundary is not past it, however far its spaces run.
        let padded = TextSpan {
            width: 8.0 + 20.0 * 3.0,
            ..span(&format!("12{}", " ".repeat(20)), 440.0, 300.0)
        };
        assert_eq!(column_holding(&bounds, &padded), Some(1));
    }
}
