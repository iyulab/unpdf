//! Tables ruled across but not down.
//!
//! The usual way to set a table in a paper or a report has no vertical rules at all: a rule
//! above the table, one under its header and one under its last row (LaTeX's `booktabs`,
//! APA style), or a rule between every row. Lattice detection needs rules on both axes and
//! never sees these; stream detection reads text only and never sees the rules. So the
//! rules here say *where* a table is — the bands between rules that share one horizontal
//! extent — and the text says *what columns* it has: the vertical gaps no line of it crosses.
//!
//! Two rules with text between them are not a table by themselves — a boxed note or a
//! pull quote is drawn the same way. A band counts only when its lines line up in columns
//! and do not read as running text, and a table is a run of such bands; a band that is a
//! caption or a paragraph ends it. A chart's gridlines are rules of one extent too, but
//! most of the bands between them hold no text: a stack like that is left alone.

use super::lattice::LatticeGrid;
use super::layout::{TextSpan, COLUMN_GAP_EM};
use super::table_detector::{
    group_into_rows, is_running_text_row, without_heading_ends, TableRowData,
};
use super::vector_graphics::GraphicsLine;

/// Shorter horizontal strokes are underlines, strike-outs or chart ticks, not table rules.
const MIN_RULE_LENGTH: f32 = 40.0;
/// How far two rules' ends may differ and still be the same table's (points).
const EXTENT_TOLERANCE: f32 = 3.0;
/// Rules closer than this are one rule drawn twice (points).
const SAME_RULE: f32 = 1.0;
/// Rules of a stack closer than this are one rule: a journal's double rule, two strokes 2–3
/// points apart, with no room between them for a line of text. pdfplumber snaps edges within
/// 3 points (`snap_tolerance`) for the same reason; kept apart, the strip between the strokes
/// is an empty band and the stack reads as a chart's gridlines.
const DOUBLE_RULE: f32 = 3.0;
/// Rows of a band are grouped like stream-mode rows: baselines within this share of the size.
const ROW_TOLERANCE: f32 = 0.5;
/// A cell this many words long is a line of prose, not a table entry.
const PROSE_WORDS: usize = 7;

/// Horizontal rules that share one extent, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuleStack {
    pub left_x: f32,
    pub right_x: f32,
    /// Rule positions, descending (PDF y grows upward).
    pub ys: Vec<f32>,
}

/// Rules drawn in pieces that meet are one rule: Word, among others, draws a table's border as
/// one filled rectangle per cell, and a joint a fraction of a point wide where two cells meet.
/// `(left, right, y)` pieces on one line (within [`SAME_RULE`]) whose ends touch (within
/// [`SAME_RULE`]) are joined; a gap wider than that keeps two rules apart.
fn joined_rules(pieces: impl Iterator<Item = (f32, f32, f32)>) -> Vec<(f32, f32, f32)> {
    let mut pieces: Vec<(f32, f32, f32)> = pieces.collect();
    pieces.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.total_cmp(&b.0)));
    let mut joined: Vec<(f32, f32, f32)> = Vec::new();
    for (left, right, y) in pieces {
        // Pieces arrive top to bottom, left to right: only a rule on this line can be joined
        // to, and the last such rule is the one that ends furthest right so far.
        let on_line = joined
            .iter_mut()
            .rev()
            .take_while(|j| (j.2 - y).abs() <= SAME_RULE)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match on_line {
            Some(rule) if left <= rule.1 + SAME_RULE => rule.1 = rule.1.max(right),
            _ => joined.push((left, right, y)),
        }
    }
    joined
}

/// Group a page's horizontal rules by the extent they share. Only stacks of two or more
/// rules can bound a table.
pub(crate) fn rule_stacks(lines: &[GraphicsLine]) -> Vec<RuleStack> {
    let mut rules = joined_rules(
        lines
            .iter()
            .filter(|l| l.ruled && (l.y0 - l.y1).abs() <= 1.0)
            .map(|l| (l.x0.min(l.x1), l.x0.max(l.x1), (l.y0 + l.y1) / 2.0)),
    );
    rules.retain(|(left, right, _)| right - left >= MIN_RULE_LENGTH);
    rules.sort_by(|a, b| b.2.total_cmp(&a.2));

    let mut stacks: Vec<RuleStack> = Vec::new();
    for (left, right, y) in rules {
        let same_extent = |s: &&mut RuleStack| {
            (s.left_x - left).abs() <= EXTENT_TOLERANCE
                && (s.right_x - right).abs() <= EXTENT_TOLERANCE
        };
        match stacks.iter_mut().find(same_extent) {
            Some(stack) => {
                if stack.ys.last().is_none_or(|&last| last - y > DOUBLE_RULE) {
                    stack.ys.push(y);
                }
            }
            None => stacks.push(RuleStack {
                left_x: left,
                right_x: right,
                ys: vec![y],
            }),
        }
    }
    stacks.retain(|s| s.ys.len() >= 2);
    stacks
}

/// The tables the rule stacks bound, as grids for [`super::lattice::build_table`]: one row per
/// line of text, one column per gap the lines leave open. A stack overlapping a ruled
/// (lattice) grid is that grid's business.
pub(crate) fn grids(
    stacks: &[RuleStack],
    spans: &[TextSpan],
    lattice: &[LatticeGrid],
) -> Vec<LatticeGrid> {
    let mut out = Vec::new();
    for stack in stacks {
        let overlaps_lattice = lattice.iter().any(|g| {
            g.left_x < stack.right_x
                && stack.left_x < g.right_x
                && g.bottom_y < stack.ys[0]
                && stack.ys[stack.ys.len() - 1] < g.top_y
        });
        if overlaps_lattice {
            continue;
        }
        // Each band between two neighbouring rules, with the lines of text inside it.
        let bands: Vec<(f32, f32, Vec<TableRowData>)> = stack
            .ys
            .windows(2)
            .map(|w| {
                let inside: Vec<TextSpan> = spans
                    .iter()
                    .filter(|s| {
                        let mid = s.x + s.width / 2.0;
                        s.y < w[0]
                            && s.y > w[1]
                            && mid >= stack.left_x - EXTENT_TOLERANCE
                            && mid <= stack.right_x + EXTENT_TOLERANCE
                            && !s.text.trim().is_empty()
                    })
                    .cloned()
                    .collect();
                (w[0], w[1], group_into_rows(&inside, ROW_TOLERANCE))
            })
            .collect();

        // A chart's gridlines: most bands between them are empty. Every band of a table holds
        // a row at least.
        let empty = bands.iter().filter(|b| b.2.is_empty()).count();
        if empty * 2 >= bands.len() {
            log::debug!(
                "ruled rows: {} rules at x {:.0}..{:.0} read as a chart's gridlines ({empty} of {} bands hold no text)",
                stack.ys.len(),
                stack.left_x,
                stack.right_x,
                bands.len()
            );
            continue;
        }

        // A table is a run of bands whose text lines up in columns.
        let mut i = 0;
        while i < bands.len() {
            if let Some((last, grid)) = record_run(&bands, i) {
                out.push(grid);
                i = last + 1;
                continue;
            }
            if !is_tabular(&bands[i].2) {
                log::debug!(
                    "ruled rows: band y {:.0}..{:.0} ({} lines) does not line up in columns or reads as prose",
                    bands[i].0,
                    bands[i].1,
                    bands[i].2.len()
                );
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < bands.len() && is_tabular(&bands[j + 1].2) {
                j += 1;
            }
            // The band right above the run may be its header, even when it does not line up
            // in columns by itself: a header of two lines, the second with one cell
            // (`Entry R Conditions Yield` over `cis:trans`), or one whose cells span columns.
            let first = if i > 0 && is_header_band(&bands[i - 1].2) {
                i - 1
            } else {
                i
            };
            let all: Vec<TableRowData> = bands[first..=j]
                .iter()
                .flat_map(|b| b.2.iter().cloned())
                .collect();
            // A section's heading under the rule its section draws is not the table's row.
            let kept = without_heading_ends(&all);
            let rows: Vec<&TableRowData> = all[kept.clone()].iter().collect();
            // The grid stops halfway to a heading it leaves out, so the heading is not binned
            // into the table's first or last row.
            let top = if kept.start > 0 {
                (all[kept.start - 1].y + all[kept.start].y) / 2.0
            } else {
                bands[first].0
            };
            let bottom = if kept.end < all.len() {
                (all[kept.end - 1].y + all[kept.end].y) / 2.0
            } else {
                bands[j].1
            };
            // Columns come from the body: the last band when the table has more than one,
            // because a header above a rule may hold cells that span several columns.
            let body: Vec<&TableRowData> = if j > i && bands[j].2.len() >= 2 {
                bands[j].2.iter().collect()
            } else if first < i {
                bands[i].2.iter().collect()
            } else {
                rows.clone()
            };
            if let Some(grid) = grid_of(stack, top, bottom, &rows, &body) {
                log::debug!(
                    "ruled rows: table y {top:.1}..{bottom:.1} from {} lines at {:?}; row bounds {:.1?}, column bounds {:.1?}",
                    rows.len(),
                    rows.iter().map(|r| r.y).collect::<Vec<_>>(),
                    grid.row_bounds,
                    grid.col_bounds
                );
                out.push(grid);
            }
            i = j + 1;
        }
    }
    out.sort_by(|a, b| b.top_y.total_cmp(&a.top_y));
    out
}

/// The most words a record's key holds: a name, an option, a reference, not a sentence.
const KEY_WORDS: usize = 5;

/// A band that is one record of a glossary-like table: its first line is a short key and the
/// text of its description, and every further line starts to the right of the key, as a wrapped
/// description does.
struct Record {
    key_right: f32,
    text_left: f32,
    /// Whether the description wraps onto lines of its own.
    wraps: bool,
}

fn record_of(rows: &[TableRowData]) -> Option<Record> {
    let first = rows.first()?;
    // Exactly a key and its description: a row of three cells is three columns, and two would
    // run the last pair together.
    let [key, text] = first.spans.as_slice() else {
        return None;
    };
    // Both are words: a chart's figures between its rules have a key and a description too.
    let worded = |s: &TextSpan| s.text.chars().filter(|c| c.is_alphabetic()).count() >= 2;
    if key.text.split_whitespace().count() > KEY_WORDS || !worded(key) || !worded(text) {
        return None;
    }
    let key_right = key.ink_extent().1;
    let gap = key.font_size * COLUMN_GAP_EM;
    if text.x - key_right < gap {
        return None;
    }
    // A wrapped description stays right of the gap; a line that comes back to the key's
    // column is another row, or a paragraph.
    let wraps = rows.len() > 1;
    if rows[1..]
        .iter()
        .any(|r| r.spans.iter().any(|s| s.x < key_right + gap))
    {
        return None;
    }
    Some(Record {
        key_right,
        text_left: text.x,
        wraps,
    })
}

/// The table made of the records that start at band `start`: bands each holding a short key and
/// a description, ruled apart, at least two and at least one of them wrapped. A band alone shows
/// no column (one line has text on both sides of the gap) and its description lines read as long
/// cells; the same gap in every band does show it. The row is the band, so a description that
/// wraps stays in its cell. A single band is a note with a label, not a table.
fn record_run(
    bands: &[(f32, f32, Vec<TableRowData>)],
    start: usize,
) -> Option<(usize, LatticeGrid)> {
    let mut records = Vec::new();
    for band in &bands[start..] {
        match record_of(&band.2) {
            Some(r) => records.push(r),
            None => break,
        }
    }
    // The description column starts at one x in every band.
    let text_left = records.first()?.text_left;
    let aligned = records
        .iter()
        .take_while(|r| (r.text_left - text_left).abs() <= EXTENT_TOLERANCE)
        .count();
    records.truncate(aligned);
    if records.len() < 2 || !records.iter().any(|r| r.wraps) {
        return None;
    }
    let last = start + records.len() - 1;
    let run = &bands[start..=last];
    let key_right = records.iter().map(|r| r.key_right).fold(f32::MIN, f32::max);

    let cells: Vec<Vec<String>> = run
        .iter()
        .map(|b| {
            let first = &b.2[0];
            let rest: Vec<&str> =
                b.2.iter()
                    .flat_map(|r| r.spans.iter())
                    .skip(1)
                    .map(|s| s.text.trim())
                    .collect();
            vec![first.spans[0].text.trim().to_string(), rest.join(" ")]
        })
        .collect();
    if super::table_detector::TableDetector::is_table_of_contents(&cells) {
        return None;
    }

    let spans = || {
        run.iter()
            .flat_map(|b| b.2.iter())
            .flat_map(|r| r.spans.iter())
    };
    let left = spans().map(|s| s.x).fold(f32::MAX, f32::min);
    let right = spans().map(|s| s.x + s.width).fold(f32::MIN, f32::max);
    let mut row_bounds: Vec<f32> = run.iter().map(|b| b.0).collect();
    row_bounds.push(run[run.len() - 1].1);
    let grid = LatticeGrid {
        top_y: run[0].0,
        bottom_y: run[run.len() - 1].1,
        left_x: left - 1.0,
        right_x: right + 1.0,
        row_bounds,
        col_bounds: vec![left - 1.0, (key_right + text_left) / 2.0, right + 1.0],
        rules: Vec::new(),
    };
    log::debug!(
        "ruled rows: {} records ruled apart, keys left of x {key_right:.0}, descriptions from x {text_left:.0}",
        run.len()
    );
    Some((last, grid))
}

/// Whether a band that is not a table by itself can head the table below it: its lines are
/// not prose, and one of them is set in cells (two runs or more, a caption is one).
fn is_header_band(rows: &[TableRowData]) -> bool {
    let refs: Vec<&TableRowData> = rows.iter().collect();
    !rows.is_empty()
        && !is_tabular(rows)
        && !reads_as_prose(&refs)
        && rows.iter().any(|r| r.spans.len() >= 2)
}

/// Whether a band's lines read as table rows: some gap stays open down all of them, and
/// they are not prose.
fn is_tabular(rows: &[TableRowData]) -> bool {
    if rows.is_empty() {
        return false;
    }
    let refs: Vec<&TableRowData> = rows.iter().collect();
    !reads_as_prose(&refs) && !column_gaps(&refs).is_empty()
}

/// Lines of running text, or cells as long as sentences.
pub(crate) fn reads_as_prose(rows: &[&TableRowData]) -> bool {
    let running = rows.iter().filter(|r| is_running_text_row(r)).count();
    let cells: Vec<usize> = rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .map(|s| s.text.split_whitespace().count())
        .collect();
    let long = cells.iter().filter(|&&w| w >= PROSE_WORDS).count();
    running * 2 > rows.len() || long * 2 > cells.len()
}

/// The vertical gaps, as `(start, end)` x ranges, that the rows leave open between two
/// columns: wide enough to separate cells, with text on both sides in two rows (or the one
/// row a header band may hold).
///
/// A row may cross a gap when the rest leave it open — a producer can draw a short cell and
/// the next one as one run of text (`montmorillonite/smectite 100`), and that run is not a
/// reason to lose the table. Up to a fifth of the rows may cross, and none in a table of
/// fewer than five rows; the run that crosses stays in the cell it starts in.
fn column_gaps(rows: &[&TableRowData]) -> Vec<(f32, f32)> {
    let mut sizes: Vec<f32> = rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .map(|s| s.font_size)
        .collect();
    if sizes.len() < 2 {
        return Vec::new();
    }
    sizes.sort_by(f32::total_cmp);
    let min_gap = sizes[sizes.len() / 2] * COLUMN_GAP_EM;
    let allowed = |n: usize| if n >= 5 { n / 5 } else { 0 };

    let mut gaps = open_gaps(rows, min_gap, allowed(rows.len()), rows.len().min(2));

    // A row that is one run of text -- a table's own title, a label that spans its columns --
    // says nothing about where the columns are, yet by crossing a gap it can close it. A gap the
    // rows set in cells keep open stands anyway when at least three of them have text on both
    // sides of it, the evidence pdfplumber asks of a text edge (`min_words_vertical`). Without
    // these rows the table would lose a column to two heading lines, as one with its title and a
    // group label did (nine lines, one allowed to cross).
    let cells: Vec<&TableRowData> = rows
        .iter()
        .copied()
        .filter(|r| r.spans.len() >= 2)
        .collect();
    let runs = rows.len() - cells.len();
    // Only a label or a title: a line of running text under the table is not one, and letting
    // it through would draw the paragraph below a table into its cells.
    let labels_only = rows.iter().filter(|r| r.spans.len() < 2).all(|r| {
        r.spans
            .iter()
            .map(|s| s.text.split_whitespace().count())
            .sum::<usize>()
            <= LABEL_WORDS
    });
    if runs > 0 && runs * 2 <= rows.len() && labels_only && cells.len() >= MIN_EDGE_ROWS {
        for gap in open_gaps(&cells, min_gap, allowed(cells.len()), MIN_EDGE_ROWS) {
            if !gaps.iter().any(|g| g.0 < gap.1 && gap.0 < g.1) {
                gaps.push(gap);
            }
        }
        gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    if gaps.is_empty() {
        gaps = worksheet_gaps(rows, min_gap);
    }
    gaps
}

/// The longest line, in words, that counts as a label or a title rather than running text.
const LABEL_WORDS: usize = 10;

/// Rows with text on both sides of a gap that let it stand without the rows that are one run of
/// text (see [`column_gaps`]).
const MIN_EDGE_ROWS: usize = 3;

/// The x ranges, as `(start, end)`, that at most `allowed` of `rows` cover, wide enough to part
/// two cells, with text on both sides of the gap in `need_both_sides` rows.
fn open_gaps(
    rows: &[&TableRowData],
    min_gap: f32,
    allowed: usize,
    need_both_sides: usize,
) -> Vec<(f32, f32)> {
    let extent = TextSpan::ink_extent;
    let left = rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .map(|s| extent(s).0)
        .fold(f32::MAX, f32::min);
    let right = rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .map(|s| extent(s).1)
        .fold(f32::MIN, f32::max);
    // How many rows cover each point of the width, in steps of half a point.
    const STEP: f32 = 0.5;
    let bins = ((right - left) / STEP).ceil().max(1.0) as usize;
    let mut covering = vec![0usize; bins];
    for row in rows {
        let mut mark = vec![false; bins];
        for span in &row.spans {
            let (a, b) = extent(span);
            let from = ((a - left) / STEP).floor().max(0.0) as usize;
            let to = (((b - left) / STEP).ceil() as usize).min(bins);
            for m in &mut mark[from..to] {
                *m = true;
            }
        }
        for (count, m) in covering.iter_mut().zip(mark) {
            *count += m as usize;
        }
    }

    let mut gaps = Vec::new();
    let mut i = 0;
    while i < bins {
        if covering[i] > allowed {
            i += 1;
            continue;
        }
        let start = i;
        while i < bins && covering[i] <= allowed {
            i += 1;
        }
        let (a, b) = (left + start as f32 * STEP, left + i as f32 * STEP);
        // Interior only, and wide enough to part two cells.
        if start == 0 || i >= bins || b - a < min_gap {
            continue;
        }
        let both_sides = rows
            .iter()
            .filter(|r| {
                r.spans.iter().any(|s| extent(s).1 <= a + STEP)
                    && r.spans.iter().any(|s| extent(s).0 >= b - STEP)
            })
            .count();
        if both_sides >= need_both_sides {
            gaps.push((a, b));
        }
    }
    gaps
}

/// The gaps between a worksheet's header cells ([`is_worksheet`]). Found from the header alone:
/// with only one row crossing to the right, the scan above lets that row cross and reads the
/// whole width beside the entries as one open edge.
fn worksheet_gaps(rows: &[&TableRowData], min_gap: f32) -> Vec<(f32, f32)> {
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    header
        .spans
        .windows(2)
        .map(|w| (w[0].ink_extent().1, w[1].x))
        .filter(|&(a, b)| b - a >= min_gap && is_worksheet(rows, (a, b)))
        .collect()
}

/// Whether rows that line up in columns are a table rather than a chart drawn between rules:
/// most rows have text on both sides of some column gap, and most carry words — a chart's
/// data labels are figures alone, its words (axis titles, the legend) sit outside the rules.
fn reads_as_table(rows: &[&TableRowData], gaps: &[(f32, f32)]) -> bool {
    let split = rows
        .iter()
        .filter(|r| {
            gaps.iter().any(|&(a, b)| {
                r.spans.iter().any(|s| s.x < a) && r.spans.iter().any(|s| s.x >= b - 0.5)
            })
        })
        .count();
    let worded = rows
        .iter()
        .filter(|r| {
            r.spans
                .iter()
                .any(|s| s.text.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        })
        .count();
    let worksheet = gaps.iter().any(|&gap| is_worksheet(rows, gap));
    (split * 2 >= rows.len() || worksheet) && worded * 2 >= rows.len()
}

/// A table to be filled in: a header row naming its columns, and rows under it that fill only
/// the columns left of `gap` -- the rest is blank for the reader to write in. Only the header
/// crosses the gap, so nothing else shows the column; three rows at least, each entry short.
fn is_worksheet(rows: &[&TableRowData], gap: (f32, f32)) -> bool {
    let [header, body @ ..] = rows else {
        return false;
    };
    let (a, b) = gap;
    rows.len() >= 3
        && header.spans.iter().any(|s| s.ink_extent().1 <= a + 0.5)
        && header.spans.iter().any(|s| s.x >= b - 0.5)
        && body.iter().all(|r| {
            r.spans
                .iter()
                .all(|s| s.ink_extent().1 <= a + 0.5 && s.text.split_whitespace().count() <= 3)
        })
}

/// The grid of one ruled table: columns at the body's gaps, a row per line of text.
fn grid_of(
    stack: &RuleStack,
    top: f32,
    bottom: f32,
    rows: &[&TableRowData],
    body: &[&TableRowData],
) -> Option<LatticeGrid> {
    let mut gaps = column_gaps(body);
    if gaps.is_empty() {
        gaps = column_gaps(rows);
    }
    if gaps.is_empty() || rows.len() < 2 || !reads_as_table(rows, &gaps) {
        log::debug!(
            "ruled rows: run y {top:.0}..{bottom:.0} is not a table ({} lines, {} column gaps)",
            rows.len(),
            gaps.len()
        );
        return None;
    }
    let left = rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .map(|s| s.x)
        .fold(stack.left_x, f32::min);
    let right = rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .map(|s| s.x + s.width)
        .fold(stack.right_x, f32::max);
    let mut col_bounds = vec![left - 1.0];
    col_bounds.extend(gaps.iter().map(|(a, b)| (a + b) / 2.0));
    col_bounds.push(right + 1.0);

    // Entries ruled off against page references are a table of contents, which is a list, as
    // stream detection reads one.
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let mut cells = vec![String::new(); col_bounds.len() - 1];
            for span in &row.spans {
                // A run that crosses a column boundary stays in the cell it starts in; a run
                // inside one cell is placed by its middle.
                let end = span.x + span.width;
                let crosses = col_bounds[1..col_bounds.len() - 1]
                    .iter()
                    .any(|&b| span.x < b && b < end);
                let at = if crosses {
                    span.x
                } else {
                    span.x + span.width / 2.0
                };
                let column = col_bounds.iter().skip(1).take_while(|&&b| at >= b).count();
                let cell = &mut cells[column.min(col_bounds.len() - 2)];
                if !cell.is_empty() {
                    cell.push(' ');
                }
                cell.push_str(span.text.trim());
            }
            cells
        })
        .collect();
    if super::table_detector::TableDetector::is_table_of_contents(&cells) {
        return None;
    }

    let mut ys: Vec<f32> = rows.iter().map(|r| r.y).collect();
    ys.sort_by(|a, b| b.total_cmp(a));
    let mut row_bounds = vec![top];
    row_bounds.extend(ys.windows(2).map(|w| (w[0] + w[1]) / 2.0));
    row_bounds.push(bottom);

    Some(LatticeGrid {
        top_y: top,
        bottom_y: bottom,
        left_x: col_bounds[0],
        right_x: col_bounds[col_bounds.len() - 1],
        row_bounds,
        col_bounds,
        // Columns found from the text, not drawn: no edge is evidence of a merge.
        rules: Vec::new(),
    })
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

    fn span(text: &str, x: f32, y: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            links: Vec::new(),
            x,
            y,
            width: text.len() as f32 * 5.0,
            width_measured: false,
            ink_width: None,
            font_size: 10.0,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
            is_struck: false,
            is_monospace: false,
            direction: TextSpan::LEFT_TO_RIGHT,
            letter_spacing: 0.0,
        }
    }

    /// A booktabs table: rules above, under the header, and under the last row.
    fn three_rule_table() -> (Vec<GraphicsLine>, Vec<TextSpan>) {
        let rules = vec![
            h(500.0, 100.0, 400.0),
            h(484.0, 100.0, 400.0),
            h(440.0, 100.0, 400.0),
        ];
        let spans = vec![
            span("Mineral", 105.0, 488.0),
            span("CEC", 300.0, 488.0),
            span("kaolinite", 105.0, 470.0),
            span("10", 300.0, 470.0),
            span("illite", 105.0, 458.0),
            span("30", 300.0, 458.0),
            span("humus", 105.0, 446.0),
            span("200", 300.0, 446.0),
        ];
        (rules, spans)
    }

    /// Word draws a table's border in one piece per cell, with a joint a half point wide where
    /// two cells meet; the pieces end where the next begins. They are one rule, as drawn.
    #[test]
    fn a_rule_drawn_in_pieces_per_cell_is_one_rule() {
        let (_, spans) = three_rule_table();
        let rules: Vec<GraphicsLine> = [500.0, 484.0, 440.0]
            .into_iter()
            .flat_map(|y| [h(y, 100.0, 250.0), h(y, 250.0, 250.4), h(y, 250.4, 400.0)])
            .collect();
        let stacks = rule_stacks(&rules);
        assert_eq!(stacks.len(), 1, "{stacks:?}");
        assert_eq!((stacks[0].left_x, stacks[0].right_x), (100.0, 400.0));
        assert_eq!(grids(&stacks, &spans, &[]).len(), 1);
    }

    /// A journal draws the rules above and under a table twice, a few points apart. The strip
    /// between the two strokes holds no text and is not a band: counted as one, two of the four
    /// bands are empty and the stack reads as a chart's gridlines.
    #[test]
    fn a_rule_drawn_twice_a_few_points_apart_is_one_rule() {
        let rules = vec![
            h(500.0, 100.0, 400.0),
            h(497.2, 100.0, 400.0),
            h(484.0, 100.0, 400.0),
            h(440.0, 100.0, 400.0),
            h(437.2, 100.0, 400.0),
        ];
        let (_, spans) = three_rule_table();
        let stacks = rule_stacks(&rules);
        assert_eq!(stacks.len(), 1, "{stacks:?}");
        assert_eq!(stacks[0].ys.len(), 3, "{stacks:?}");
        let grids = grids(&stacks, &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        let (table, consumed) = super::super::lattice::build_table(&grids[0], &spans).unwrap();
        assert_eq!(consumed.len(), spans.len());
        assert_eq!(table.rows.len(), 4);
    }

    /// A double rule between the header and the body does not cut the table in two.
    #[test]
    fn a_double_rule_under_the_header_does_not_split_the_table() {
        let rules = vec![
            h(500.0, 100.0, 400.0),
            h(484.0, 100.0, 400.0),
            h(481.2, 100.0, 400.0),
            h(440.0, 100.0, 400.0),
        ];
        let (_, spans) = three_rule_table();
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        assert_eq!((grids[0].top_y, grids[0].bottom_y), (500.0, 440.0));
    }

    /// A column of text beside the table, its baselines a few points off the table's, must not
    /// pull a row of the table into the line above: the lines are the table's own, not the page's.
    #[test]
    fn text_beside_a_table_does_not_move_its_rows() {
        let (rules, mut spans) = three_rule_table();
        let mut aside = span("beside", 420.0, 462.0);
        aside.font_size = 12.0;
        spans.push(aside);
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        let (table, _) = super::super::lattice::build_table(&grids[0], &spans).unwrap();
        let first_column: Vec<String> =
            table.rows.iter().map(|r| r.cells[0].plain_text()).collect();
        assert_eq!(first_column, ["Mineral", "kaolinite", "illite", "humus"]);
    }

    /// A rule between records, each a short key and a description that wraps under its own first
    /// line: a glossary, a list of options. A band holds one line with text on both sides of the
    /// gap, the rest start to its right, so the band alone shows no column and the description
    /// lines read as long cells; the same gap in every band does show it. The row is the band.
    fn glossary() -> (Vec<GraphicsLine>, Vec<TextSpan>) {
        let rules = [500.0, 484.0, 452.0, 420.0, 388.0]
            .into_iter()
            .map(|y| h(y, 100.0, 460.0))
            .collect();
        let spans = vec![
            span("Option", 105.0, 488.0),
            span("Description", 250.0, 488.0),
            span("Accessibility", 105.0, 470.0),
            span(
                "Turns on the mode that lets you drive the page",
                250.0,
                470.0,
            ),
            span("from the keyboard only", 250.0, 458.0),
            span("Email", 105.0, 438.0),
            span("Reserved for future use and not yet working", 250.0, 438.0),
            span("in this release", 250.0, 426.0),
            span("Password", 105.0, 406.0),
            span("Use the Password tab to change your password", 250.0, 406.0),
            span("or reset it from the sign in page", 250.0, 394.0),
        ];
        (rules, spans)
    }

    #[test]
    fn records_ruled_apart_with_wrapped_descriptions_are_a_table() {
        let (rules, spans) = glossary();
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        let (table, consumed) = super::super::lattice::build_table(&grids[0], &spans).unwrap();
        assert_eq!(consumed.len(), spans.len());
        let rows: Vec<Vec<String>> = table
            .rows
            .iter()
            .map(|r| r.cells.iter().map(|c| c.plain_text()).collect())
            .collect();
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[0], ["Option", "Description"]);
        assert_eq!(rows[1][0], "Accessibility");
        assert!(
            rows[1][1].contains("drive the page") && rows[1][1].contains("keyboard only"),
            "{rows:?}"
        );
        assert_eq!(rows[3][0], "Password");
    }

    /// One boxed note with a label hanging beside its paragraph is drawn the same way, and is
    /// not a table: a record is evidence only in a run of them.
    #[test]
    fn a_single_labelled_paragraph_between_rules_is_not_a_table() {
        let rules = vec![h(500.0, 100.0, 460.0), h(430.0, 100.0, 460.0)];
        let spans = vec![
            span("NOTE", 105.0, 488.0),
            span(
                "Turns on the mode that lets you drive the page",
                250.0,
                488.0,
            ),
            span(
                "from the keyboard only and is meant for those",
                250.0,
                476.0,
            ),
            span("who cannot use a pointing device at all", 250.0, 464.0),
            span("and for nobody else on this release", 250.0, 452.0),
        ];
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    /// A chart's figures between its rules have a «key» and a «description» too, and no words.
    #[test]
    fn figures_ruled_apart_are_not_records() {
        let rules: Vec<GraphicsLine> = [500.0, 460.0, 420.0, 380.0]
            .into_iter()
            .map(|y| h(y, 100.0, 460.0))
            .collect();
        let spans = vec![
            span("32", 105.0, 488.0),
            span("37 20", 250.0, 488.0),
            span("40", 250.0, 476.0),
            span("30", 105.0, 448.0),
            span("17 21", 250.0, 448.0),
            span("46", 250.0, 436.0),
            span("20", 105.0, 408.0),
            span("38 22", 250.0, 408.0),
            span("19", 250.0, 396.0),
        ];
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    /// A row of three cells is a table of three columns, not a key and its description: reading
    /// it as two would run the last two together.
    #[test]
    fn a_row_of_three_cells_is_not_a_record() {
        let rules: Vec<GraphicsLine> = [500.0, 460.0, 420.0, 380.0]
            .into_iter()
            .map(|y| h(y, 100.0, 460.0))
            .collect();
        let spans = vec![
            span("Salt", 105.0, 480.0),
            span("Sea salt", 200.0, 480.0),
            span("Storms over the sea", 330.0, 480.0),
            span("and spray", 330.0, 468.0),
            span("Nitric acid", 105.0, 440.0),
            span("Lightning", 200.0, 440.0),
            span("Forest fires", 330.0, 440.0),
            span("and traffic", 330.0, 428.0),
            span("Heavy metals", 105.0, 400.0),
            span("Mining", 200.0, 400.0),
            span("Industry", 330.0, 400.0),
            span("and waste", 330.0, 388.0),
        ];
        // Whatever reads it, no grid of two columns.
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert!(grids.iter().all(|g| g.col_bounds.len() >= 4), "{grids:?}");
    }

    /// Paragraphs ruled apart that each start at the left are not records: no short key on its
    /// own, no gap the lines leave open.
    #[test]
    fn paragraphs_ruled_apart_are_not_records() {
        let rules: Vec<GraphicsLine> = [500.0, 460.0, 420.0, 380.0]
            .into_iter()
            .map(|y| h(y, 100.0, 460.0))
            .collect();
        let spans = vec![
            span(
                "Turns on the mode that lets you drive the page",
                105.0,
                480.0,
            ),
            span(
                "from the keyboard only and is meant for those",
                105.0,
                468.0,
            ),
            span(
                "who cannot use a pointing device at all today",
                105.0,
                440.0,
            ),
            span("and for nobody else on this release of it", 105.0, 428.0),
            span(
                "A third paragraph of ordinary running text here",
                105.0,
                400.0,
            ),
            span("and a fourth line that goes on and on", 105.0, 388.0),
        ];
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    /// Rules a line of text can fit between stay apart: a table ruled between its rows.
    #[test]
    fn rules_a_line_of_text_apart_are_not_merged() {
        let rules: Vec<GraphicsLine> = [500.0, 488.0, 476.0, 464.0]
            .into_iter()
            .map(|y| h(y, 100.0, 400.0))
            .collect();
        assert_eq!(rule_stacks(&rules)[0].ys.len(), 4);
    }

    /// A contents list ruled between its entries is still a list: entries against page numbers
    /// that never go down.
    #[test]
    fn a_ruled_table_of_contents_is_not_a_table() {
        let rules: Vec<GraphicsLine> = [500.0, 480.0, 460.0, 440.0, 420.0]
            .into_iter()
            .map(|y| h(y, 100.0, 400.0))
            .collect();
        let entries = [
            ("Executive Summary", "4"),
            ("Legal Framework", "6"),
            ("Election Administration", "11"),
            ("Civil Society Engagement", "15"),
        ];
        let spans = |second: &[&str]| -> Vec<TextSpan> {
            entries
                .iter()
                .zip(second)
                .enumerate()
                .flat_map(|(i, ((name, _), value))| {
                    let y = 486.0 - i as f32 * 20.0;
                    [span(name, 105.0, y), span(value, 380.0, y)]
                })
                .collect()
        };
        let pages = spans(&["4", "6", "11", "15"]);
        assert!(grids(&rule_stacks(&rules), &pages, &[]).is_empty());
        // The same layout with words in the second column is a table.
        let words = spans(&["Annex", "Note", "Index", "Other"]);
        assert_eq!(grids(&rule_stacks(&rules), &words, &[]).len(), 1);
    }

    /// A header of two lines, the second with one cell, does not line up in columns by itself
    /// (the second line crosses no gap) but heads the table under it.
    #[test]
    fn a_two_line_header_with_a_sparse_second_line_stays_in_its_table() {
        let rules = vec![
            h(500.0, 100.0, 400.0),
            h(460.0, 100.0, 400.0),
            h(400.0, 100.0, 400.0),
        ];
        let spans = vec![
            span("Entry", 105.0, 488.0),
            span("Yield", 300.0, 488.0),
            span("cis:trans", 300.0, 474.0),
            span("alpha", 105.0, 448.0),
            span("71", 300.0, 448.0),
            span("beta", 105.0, 436.0),
            span("85", 300.0, 436.0),
            span("gamma", 105.0, 424.0),
            span("62", 300.0, 424.0),
        ];
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        assert_eq!(grids[0].top_y, 500.0);
        let (table, consumed) = super::super::lattice::build_table(&grids[0], &spans).unwrap();
        assert_eq!(consumed.len(), spans.len());
        let first: Vec<String> = table.rows[0].cells.iter().map(|c| c.plain_text()).collect();
        assert_eq!(first, ["Entry", "Yield"]);
    }

    /// A caption over a table is one run of text, not a header band.
    #[test]
    fn a_caption_over_a_ruled_table_is_not_its_header() {
        let (rules, spans) = three_rule_table();
        let mut rules = rules;
        rules.insert(0, h(540.0, 100.0, 400.0));
        let mut spans = spans;
        spans.push(span("Table 1", 105.0, 520.0));
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert!(grids.iter().all(|g| g.top_y <= 500.0), "{grids:?}");
    }

    /// Rules with a real gap between them are two rules: two tables side by side, not one.
    #[test]
    fn rules_with_a_gap_between_them_stay_apart() {
        let rules: Vec<GraphicsLine> = [500.0, 440.0]
            .into_iter()
            .flat_map(|y| [h(y, 100.0, 240.0), h(y, 260.0, 400.0)])
            .collect();
        assert_eq!(rule_stacks(&rules).len(), 2);
    }

    #[test]
    fn a_table_ruled_only_across_is_a_grid() {
        let (rules, spans) = three_rule_table();
        let stacks = rule_stacks(&rules);
        assert_eq!(stacks.len(), 1);
        let grids = grids(&stacks, &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        let grid = &grids[0];
        assert_eq!((grid.row_count(), grid.column_count()), (4, 2));
        assert_eq!((grid.top_y, grid.bottom_y), (500.0, 440.0));

        let (table, consumed) = super::super::lattice::build_table(grid, &spans).unwrap();
        assert_eq!(consumed.len(), spans.len());
        let cells: Vec<Vec<String>> = table
            .rows
            .iter()
            .map(|r| r.cells.iter().map(|c| c.plain_text()).collect())
            .collect();
        assert_eq!(cells[0], ["Mineral", "CEC"]);
        assert_eq!(cells[3], ["humus", "200"]);
    }

    /// A table to fill in, ruled above and below: a header naming two columns, entries in the
    /// first, the second left blank.
    fn worksheet(header_gap: f32) -> (Vec<GraphicsLine>, Vec<TextSpan>) {
        let rules = vec![h(500.0, 100.0, 400.0), h(420.0, 100.0, 400.0)];
        let header = span("Added cation ", 105.0, 488.0);
        let second_x = 105.0 + header.width - 2.5 + header_gap;
        let spans = vec![
            header,
            span("Settling Rates of Floccules", second_x, 488.0),
            span("K+", 105.0, 474.0),
            span("Na+", 105.0, 462.0),
            span("Ca2+", 105.0, 450.0),
            span("Check", 105.0, 438.0),
        ];
        (rules, spans)
    }

    #[test]
    fn a_header_over_a_blank_column_is_a_worksheet_table() {
        // Nine points of ink-to-ink gap: under one em, over the column threshold -- but only
        // once the header cell's trailing space is not counted as ink.
        let (rules, spans) = worksheet(9.0);
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        assert_eq!((grids[0].row_count(), grids[0].column_count()), (5, 2));
        let (table, _) = super::super::lattice::build_table(&grids[0], &spans).unwrap();
        assert_eq!(
            table.to_csv(),
            "Added cation,Settling Rates of Floccules\r\nK+,\r\nNa+,\r\nCa2+,\r\nCheck,\r\n"
        );
    }

    #[test]
    fn a_header_over_a_column_of_sentences_is_not_a_worksheet() {
        let (rules, mut spans) = worksheet(9.0);
        spans[2].text = "Potassium settles slowly in the clay".to_string();
        spans[2].width = spans[2].text.len() as f32 * 5.0;
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    #[test]
    fn prose_between_two_rules_is_not_a_table() {
        let rules = vec![h(500.0, 100.0, 400.0), h(440.0, 100.0, 400.0)];
        let spans = vec![
            span(
                "A boxed note says something worth reading here",
                105.0,
                488.0,
            ),
            span("and goes on for another line of the same", 105.0, 476.0),
            span("paragraph before it ends.", 105.0, 464.0),
        ];
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    #[test]
    fn two_columns_of_prose_between_rules_are_not_a_table() {
        // A page's running head rule and footer rule bound both text columns; the gutter is
        // a gap down every line, but the cells are sentences.
        let rules = vec![h(760.0, 50.0, 550.0), h(60.0, 50.0, 550.0)];
        let mut spans = Vec::new();
        for i in 0..6 {
            let y = 700.0 - i as f32 * 12.0;
            spans.push(span(
                "the left column carries its own running text",
                55.0,
                y,
            ));
            spans.push(span("while the right column carries another one", 310.0, y));
        }
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    #[test]
    fn a_caption_between_two_tables_on_one_extent_splits_them() {
        // Two booktabs tables of the same width, the second one's caption between them.
        let mut rules = Vec::new();
        for y in [500.0, 484.0, 452.0, 400.0, 384.0, 352.0] {
            rules.push(h(y, 100.0, 400.0));
        }
        let mut spans = Vec::new();
        for (top, name) in [(500.0, "first"), (400.0, "second")] {
            spans.push(span("Name", 105.0, top - 12.0));
            spans.push(span("Value", 300.0, top - 12.0));
            spans.push(span(name, 105.0, top - 28.0));
            spans.push(span("1", 300.0, top - 28.0));
            spans.push(span("other", 105.0, top - 42.0));
            spans.push(span("2", 300.0, top - 42.0));
        }
        spans.push(span(
            "Table 2: The second table, measured the same way as the first one",
            105.0,
            430.0,
        ));
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        let bounds: Vec<(f32, f32)> = grids.iter().map(|g| (g.top_y, g.bottom_y)).collect();
        assert_eq!(bounds, [(500.0, 452.0), (400.0, 352.0)]);
    }

    #[test]
    fn rules_inside_a_ruled_grid_are_left_to_it() {
        let (rules, spans) = three_rule_table();
        let lattice = LatticeGrid {
            top_y: 510.0,
            bottom_y: 430.0,
            left_x: 90.0,
            right_x: 410.0,
            row_bounds: vec![510.0, 470.0, 430.0],
            col_bounds: vec![90.0, 250.0, 410.0],
            rules: Vec::new(),
        };
        assert!(grids(&rule_stacks(&rules), &spans, &[lattice]).is_empty());
    }

    #[test]
    fn short_strokes_and_single_rules_bound_nothing() {
        // An underline and a lone separator rule.
        let rules = vec![
            h(500.0, 100.0, 130.0),
            h(480.0, 100.0, 130.0),
            h(300.0, 50.0, 550.0),
        ];
        assert!(rule_stacks(&rules).is_empty());
    }

    #[test]
    fn a_charts_gridlines_are_not_a_table() {
        // Six gridlines of one extent; data labels sit in two of the five bands.
        let rules: Vec<GraphicsLine> = (0..6)
            .map(|i| h(500.0 - i as f32 * 20.0, 100.0, 400.0))
            .collect();
        let spans = vec![
            span("40", 150.0, 470.0),
            span("81", 250.0, 470.0),
            span("73", 350.0, 470.0),
            span("12", 150.0, 430.0),
            span("9", 250.0, 430.0),
        ];
        assert!(grids(&rule_stacks(&rules), &spans, &[]).is_empty());
    }

    #[test]
    fn one_row_drawn_across_the_column_gap_keeps_the_table() {
        // A producer drew `montmorillonite/smectite 100` as one run, across the gap every
        // other row leaves open between the columns.
        let rules = vec![h(500.0, 100.0, 400.0), h(400.0, 100.0, 400.0)];
        let mut spans = Vec::new();
        for (i, (name, value)) in [
            ("kaolinite", "10"),
            ("illite", "30"),
            ("vermiculite", "150"),
            ("humus", "200"),
            ("chlorite", "40"),
        ]
        .iter()
        .enumerate()
        {
            let y = 490.0 - i as f32 * 14.0;
            spans.push(span(name, 105.0, y));
            spans.push(span(value, 300.0, y));
        }
        let mut long = span("montmorillonite/smectite 100", 105.0, 410.0);
        long.width = 210.0;
        spans.push(long);
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1, "{grids:?}");
        assert_eq!((grids[0].row_count(), grids[0].column_count()), (6, 2));
    }

    /// A specification sheet: a group heading and a label that span the table, over a header and
    /// rows of three cells. The two spanning lines cross the gap between the first two columns,
    /// and in a table of nine lines one line may cross -- the heading rows used to close the
    /// column. Lines that are one run say nothing about where the columns are.
    fn specification_sheet(heading_rows: usize) -> Vec<TextSpan> {
        let mut spans = Vec::new();
        let mut y = 590.0;
        if heading_rows >= 1 {
            spans.push(span("GENERAL DESCRIPTION OF THE PRODUCT", 105.0, y));
            y -= 14.0;
        }
        if heading_rows >= 2 {
            spans.push(span("Wettable Powder Formulation", 105.0, y));
            y -= 14.0;
        }
        for (a, b, c) in [
            ("CHARACTERISTIC", "SPECIFICATION", "METHOD"),
            ("AI, as Methomyl", "87.5 - 92.5", "HPLC"),
            ("pH in water", "4.0 - 8.0", "pH-meter"),
            ("Appearance", "White", "Visual"),
        ] {
            y -= 12.0;
            spans.push(span(a, 105.0, y));
            spans.push(span(b, 230.0, y));
            spans.push(span(c, 340.0, y));
        }
        spans
    }

    #[test]
    fn headings_that_span_the_columns_do_not_close_a_column() {
        let rules = vec![h(610.0, 100.0, 450.0), h(400.0, 100.0, 450.0)];
        let spans = specification_sheet(2);
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert_eq!(grids.len(), 1);
        assert_eq!(
            grids[0].col_bounds.len(),
            4,
            "three columns: {:?}",
            grids[0].col_bounds
        );
    }

    #[test]
    fn rows_set_in_cells_are_what_open_a_column() {
        // Three rows in cells hold both gaps open; one spanning line is no veto.
        let rules = vec![h(610.0, 100.0, 450.0), h(400.0, 100.0, 450.0)];
        let one = grids(&rule_stacks(&rules), &specification_sheet(1), &[]);
        assert_eq!(one[0].col_bounds.len(), 4);
        // Without spanning lines nothing changed.
        let none = grids(&rule_stacks(&rules), &specification_sheet(0), &[]);
        assert_eq!(none[0].col_bounds.len(), 4);
    }

    #[test]
    fn lines_of_one_run_alone_do_not_make_columns() {
        // Only two rows are set in cells, among several that are one run: not enough to hold a
        // gap against the lines that cross it.
        let rules = vec![h(610.0, 100.0, 450.0), h(400.0, 100.0, 450.0)];
        let spans = vec![
            span("GENERAL DESCRIPTION", 105.0, 590.0),
            span("Wettable Powder Formulation", 105.0, 576.0),
            span("A long label across", 105.0, 562.0),
            span("one", 105.0, 548.0),
            span("left", 105.0, 520.0),
            span("right", 340.0, 520.0),
            span("second", 105.0, 508.0),
            span("pair", 340.0, 508.0),
        ];
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        assert!(
            grids.iter().all(|g| g.col_bounds.len() <= 3),
            "no third column from two rows: {:?}",
            grids.iter().map(|g| &g.col_bounds).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_paragraph_under_the_table_is_not_drawn_into_it() {
        // The relaxation is for labels and titles: lines of running text between the rules
        // still close the column they cross.
        let rules = vec![h(610.0, 100.0, 450.0), h(400.0, 100.0, 450.0)];
        let mut spans = specification_sheet(0);
        for (i, line) in [
            "Another way to represent the two rates of return measures is to draw a scatter graph",
            "We may visually observe how the two sets of rates of return move together by drawing",
        ]
        .iter()
        .enumerate()
        {
            spans.push(span(line, 105.0, 520.0 - i as f32 * 12.0));
        }
        let grids = grids(&rule_stacks(&rules), &spans, &[]);
        let cells: usize = grids.iter().map(|g| g.col_bounds.len()).sum();
        assert!(
            grids.is_empty() || cells <= 4,
            "paragraph lines must not become rows of a wider grid: {:?}",
            grids.iter().map(|g| &g.col_bounds).collect::<Vec<_>>()
        );
    }
}
