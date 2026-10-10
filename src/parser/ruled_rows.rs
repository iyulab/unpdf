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
use super::layout::TextSpan;
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
/// Rows of a band are grouped like stream-mode rows: baselines within this share of the size.
const ROW_TOLERANCE: f32 = 0.5;
/// The narrowest gap, in multiples of the font size, that separates two columns.
const COLUMN_GAP_EM: f32 = 0.8;
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

/// Group a page's horizontal rules by the extent they share. Only stacks of two or more
/// rules can bound a table.
pub(crate) fn rule_stacks(lines: &[GraphicsLine]) -> Vec<RuleStack> {
    let mut rules: Vec<(f32, f32, f32)> = lines
        .iter()
        .filter(|l| l.ruled && (l.y0 - l.y1).abs() <= 1.0)
        .map(|l| (l.x0.min(l.x1), l.x0.max(l.x1), (l.y0 + l.y1) / 2.0))
        .filter(|(left, right, _)| right - left >= MIN_RULE_LENGTH)
        .collect();
    rules.sort_by(|a, b| b.2.total_cmp(&a.2));

    let mut stacks: Vec<RuleStack> = Vec::new();
    for (left, right, y) in rules {
        let same_extent = |s: &&mut RuleStack| {
            (s.left_x - left).abs() <= EXTENT_TOLERANCE
                && (s.right_x - right).abs() <= EXTENT_TOLERANCE
        };
        match stacks.iter_mut().find(same_extent) {
            Some(stack) => {
                if stack.ys.last().is_none_or(|&last| last - y > SAME_RULE) {
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
            continue;
        }

        // A table is a run of bands whose text lines up in columns.
        let mut i = 0;
        while i < bands.len() {
            if !is_tabular(&bands[i].2) {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < bands.len() && is_tabular(&bands[j + 1].2) {
                j += 1;
            }
            let all: Vec<TableRowData> = bands[i..=j]
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
                bands[i].0
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
            } else {
                rows.clone()
            };
            if let Some(grid) = grid_of(stack, top, bottom, &rows, &body) {
                out.push(grid);
            }
            i = j + 1;
        }
    }
    out.sort_by(|a, b| b.top_y.total_cmp(&a.top_y));
    out
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
fn reads_as_prose(rows: &[&TableRowData]) -> bool {
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
    let extent = span_extent;
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
    let allowed = if rows.len() >= 5 { rows.len() / 5 } else { 0 };

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
        if both_sides >= rows.len().min(2) {
            gaps.push((a, b));
        }
    }
    if gaps.is_empty() {
        gaps = worksheet_gaps(rows, min_gap);
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
        .map(|w| (span_extent(&w[0]).1, w[1].x))
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

/// Where a run of text ends in ink: its trailing spaces are advance, and a gap measured to them
/// comes out a space short (a header cell drawn as `Added cation ` stood 5 pt from the next one
/// instead of 7).
fn span_extent(s: &TextSpan) -> (f32, f32) {
    let width = if s.width > 0.0 {
        s.width
    } else {
        s.text.chars().count() as f32 * s.font_size * 0.5
    };
    let trailing = s.text.len() - s.text.trim_end().len();
    let ink = (width - trailing as f32 * s.font_size * SPACE_EM).max(0.0);
    (s.x, s.x + ink)
}

/// The advance of a space, in multiples of the font size -- what most text faces draw.
const SPACE_EM: f32 = 0.25;

/// A table to be filled in: a header row naming its columns, and rows under it that fill only
/// the columns left of `gap` -- the rest is blank for the reader to write in. Only the header
/// crosses the gap, so nothing else shows the column; three rows at least, each entry short.
fn is_worksheet(rows: &[&TableRowData], gap: (f32, f32)) -> bool {
    let [header, body @ ..] = rows else {
        return false;
    };
    let (a, b) = gap;
    rows.len() >= 3
        && header.spans.iter().any(|s| span_extent(s).1 <= a + 0.5)
        && header.spans.iter().any(|s| s.x >= b - 0.5)
        && body.iter().all(|r| {
            r.spans
                .iter()
                .all(|s| span_extent(s).1 <= a + 0.5 && s.text.split_whitespace().count() <= 3)
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
            x,
            y,
            width: text.len() as f32 * 5.0,
            width_measured: false,
            font_size: 10.0,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
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
}
