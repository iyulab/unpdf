//! Which column a table cell's text belongs to, judged by where the column's text lies.
//!
//! Ruled grids and tables without rules both place a span by the column it starts in. That
//! misplaces text that starts in one column and sits over the next: a header centred over a
//! column of figures set flush right begins in the channel before the figures, left of the
//! boundary the rows below define. Where the boundary is drawn says little; where the
//! column's text lies in the other rows says what the span is over.

/// Where every column's text lies, row by row, so a span can be measured against the text
/// of the other rows of its table.
pub(crate) struct ColumnText {
    /// For each column, `(row, left, right)` of every span placed in it.
    columns: Vec<Vec<(usize, f32, f32)>>,
}

impl ColumnText {
    pub(crate) fn new(column_count: usize) -> Self {
        Self {
            columns: vec![Vec::new(); column_count],
        }
    }

    /// Record a span of `row` placed in `column`, from `left` to `right`.
    ///
    /// The placement recorded is the builder's own first answer, not merely the column the
    /// span starts in: a column of figures whose cells start in the channel before it, and
    /// are put in it by that answer, is that column's text. Recorded by where they start,
    /// those cells would be the text of the column before, and every header over them would
    /// follow them there.
    pub(crate) fn add(&mut self, row: usize, column: usize, left: f32, right: f32) {
        if let Some(spans) = self.columns.get_mut(column) {
            spans.push((row, left, right));
        }
    }

    /// How many rows other than `row` have text in `column` that `left..right` lies over —
    /// leaving out, when `past` is given, text that runs on past it.
    ///
    /// Rows are counted, not points of overlap. And text placed in a column that runs on past
    /// its boundary with the next is not that column's: it is a header spanning both
    /// (`SHIPCALLS` over `Foreign` and `Domestic`, `Aboriginal` over `MALE` and `FEMALE`), and
    /// counted as the column's text it would draw every header beneath it back. Text placed
    /// in the next column that starts before the boundary is that column's — figures set
    /// flush right start wherever their width puts them.
    fn rows_under(
        &self,
        column: usize,
        row: usize,
        past: Option<f32>,
        left: f32,
        right: f32,
    ) -> usize {
        let Some(spans) = self.columns.get(column) else {
            return 0;
        };
        let mut rows: Vec<usize> = spans
            .iter()
            .filter(|&&(r, _, rt)| r != row && past.is_none_or(|edge| rt <= edge))
            .filter(|&&(_, l, rt)| right.min(rt) > left.max(l))
            .map(|&(r, _, _)| r)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows.len()
    }

    /// Whether `left` lies within the extent of the text that rows other than `row` have in
    /// `column` -- from where an entry starts to where it ends. Text that runs on past `edge`,
    /// the column's boundary with the next, is a heading over both columns and not the
    /// column's entry (see [`Self::rows_under`]).
    fn starts_within(&self, column: usize, row: usize, left: f32, edge: f32) -> bool {
        self.columns.get(column).is_some_and(|spans| {
            spans
                .iter()
                .any(|&(r, l, rt)| r != row && rt <= edge && l - 0.5 <= left && left < rt)
        })
    }

    /// The column of a span of `row`, from `left` to `right`, that starts in column `start`,
    /// whose boundary with the next column is `edge`: the next column when the span lies
    /// over that column's text in more of the other rows than over its own column's text;
    /// its own column when it lies over its own in as many or more. When it lies over
    /// neither, the text gives no answer and `otherwise` stands — as it does in the last
    /// column, which has no boundary after it.
    ///
    /// Only the next column is a candidate: a span is moved across the one boundary it starts
    /// short of, never further, and never back.
    pub(crate) fn column_for(
        &self,
        row: usize,
        start: usize,
        edge: Option<f32>,
        left: f32,
        right: f32,
        otherwise: usize,
    ) -> usize {
        let Some(edge) = edge else {
            return otherwise;
        };
        // A span that starts within its own column's text is set in that column, however far
        // it runs on: a row label or a group heading that spans the table begins where the
        // column's entries begin. Only text that starts short of the column's entries -- in the
        // channel before them -- is read as belonging to the column it lies over.
        if self.starts_within(start, row, left, edge) {
            return start;
        }
        let here = self.rows_under(start, row, Some(edge), left, right);
        let next = self.rows_under(start + 1, row, None, left, right);
        if here == 0 && next == 0 {
            otherwise
        } else if next > here {
            start + 1
        } else {
            start
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of ports: `Foreign` and `Domestic` head columns of figures set flush right,
    /// each starting in the channel before its figures. The boundaries fall at 182 and 246.
    fn ports() -> ColumnText {
        let mut text = ColumnText::new(3);
        for row in 1..4 {
            text.add(row, 0, 77.4, 115.3);
            text.add(row, 1, 203.4, 225.8);
            text.add(row, 2, 267.0, 297.7);
        }
        text
    }

    #[test]
    fn a_header_over_the_next_columns_figures_goes_with_them() {
        let text = ports();
        // `Domestic` starts in column 1 and lies over column 2's figures only.
        assert_eq!(text.column_for(0, 1, Some(246.0), 236.6, 278.7, 1), 2);
        // `Foreign` starts in column 0 and lies over column 1's figures.
        assert_eq!(text.column_for(0, 0, Some(182.0), 178.5, 212.2, 0), 1);
    }

    /// Figures set flush right in the next column start before its boundary: they are its
    /// text all the same.
    #[test]
    fn figures_starting_before_the_boundary_are_the_next_columns_text() {
        let mut text = ColumnText::new(3);
        for row in 1..4 {
            text.add(row, 1, 141.0, 163.2);
            text.add(row, 2, 225.0, 265.1);
        }
        // Column 2 starts at 230; `Unadjusted Basis` starts in column 1.
        assert_eq!(text.column_for(0, 1, Some(230.0), 184.7, 265.1, 1), 2);
    }

    /// A header spanning both columns, in a row of its own, is neither column's text.
    #[test]
    fn a_header_spanning_the_boundary_is_not_evidence() {
        let mut text = ports();
        text.add(4, 1, 215.3, 263.2);
        text.add(5, 1, 210.0, 250.0);
        assert_eq!(text.column_for(0, 1, Some(246.0), 236.6, 278.7, 1), 2);
        // With only spanning headers above it, a header over no figures keeps its place.
        assert_eq!(text.column_for(0, 1, Some(246.0), 228.0, 262.0, 2), 2);
    }

    #[test]
    fn a_span_over_its_own_columns_text_stays() {
        let text = ports();
        assert_eq!(text.column_for(0, 1, Some(246.0), 200.0, 240.0, 2), 1);
    }

    #[test]
    fn a_span_over_no_columns_text_keeps_the_other_rule() {
        let text = ports();
        assert_eq!(text.column_for(0, 1, Some(246.0), 230.0, 262.0, 2), 2);
        assert_eq!(text.column_for(0, 1, Some(246.0), 230.0, 262.0, 1), 1);
    }

    /// The span's own row is not part of the evidence: in it, the span itself would be the
    /// text of the column it starts in.
    #[test]
    fn the_spans_own_row_is_not_evidence() {
        let mut text = ports();
        text.add(0, 1, 236.6, 278.7);
        assert_eq!(text.column_for(0, 1, Some(246.0), 236.6, 278.7, 1), 2);
    }

    #[test]
    fn the_last_column_has_no_next() {
        let text = ports();
        assert_eq!(text.column_for(0, 2, None, 270.0, 300.0, 2), 2);
    }

    /// A group heading that runs across the table starts where the first column's entries
    /// start: it is that column's, although it lies over the next column's text as well.
    #[test]
    fn a_span_starting_within_its_columns_text_stays_however_far_it_runs() {
        let mut text = ColumnText::new(3);
        for row in 1..5 {
            text.add(row, 0, 94.0, 160.0);
            text.add(row, 1, 209.0, 270.0);
            text.add(row, 2, 323.0, 400.0);
        }
        // Starts at 94, runs to 290: over column 1's text in every row, yet begins in column 0's.
        assert_eq!(text.column_for(0, 0, Some(193.0), 94.0, 290.0, 0), 0);
        // Begun in the channel before column 1's entries, the same span goes with them.
        assert_eq!(text.column_for(0, 0, Some(193.0), 180.0, 290.0, 0), 1);
    }
}
