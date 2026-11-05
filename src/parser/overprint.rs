//! Text drawn twice at the same place is one text.
//!
//! A producer that wants bold, or a shadow, without a bold face can show a string again on
//! top of itself — the same glyphs, the same size, a fraction of a point apart (`0` apart for
//! the usual overprint, up to about a tenth of an em for a shadow). The page shows one word;
//! the content stream holds two runs, and reading both gives `CollectionCollection`.
//!
//! Apache PDFBox drops such runs by default (`PDFTextStripper.suppressDuplicateOverlappingText`,
//! on unless turned off) and pdfplumber offers the same as `dedupe_chars`; neither one reads a
//! word twice. The later run is the one dropped, so the reading is the one the stream draws
//! first.
//!
//! Only runs that agree on *everything that makes them the same drawing* go: the text, the
//! font, the size, the direction of the baseline, and the place. Two equal strings anywhere
//! else on a page — a column of `0`, a repeated label — are different marks.

use super::layout::TextSpan;
use std::collections::HashMap;

/// How far apart, in ems, two runs may stand and still be one drawing: the widest shadow
/// offset measured on the public corpus is 0.08 em (0.9 pt at 11 pt); an exact overprint is 0.
const OVERPRINT_OFFSET_EM: f32 = 0.1;

/// How much two sizes may differ, as a share of the size, for the runs to be one drawing.
const OVERPRINT_SIZE_SLACK: f32 = 0.05;

/// The reach is measured on sizes up to this (points), and runs smaller than the floor are
/// not compared at all: both bound the work a page can ask for — the cost of a run is the
/// number of cells around it, and a speck-sized run would stand in a crowd of its own.
const REACH_SIZE_CAP: f32 = 72.0;
const SIZE_FLOOR: f32 = 1.0;

/// Side of a grid cell (points). A run is compared only with the runs kept in the cells its
/// reach touches.
const CELL: f32 = 1.0;

/// At most this many kept runs of one cell are compared with a new run, newest first: runs
/// that differ only in font name can crowd a cell, and the work must not grow with the crowd.
const MAX_COMPARED: usize = 16;

fn reach_of(span: &TextSpan) -> f32 {
    OVERPRINT_OFFSET_EM * span.font_size.min(REACH_SIZE_CAP)
}

/// Whether `later` is `earlier` drawn again on top of itself.
fn is_overprint(earlier: &TextSpan, later: &TextSpan) -> bool {
    let reach = reach_of(earlier);
    earlier.text == later.text
        && earlier.font_name == later.font_name
        && earlier.direction == later.direction
        && (earlier.font_size - later.font_size).abs() <= OVERPRINT_SIZE_SLACK * earlier.font_size
        && (earlier.x - later.x).abs() <= reach
        && (earlier.y - later.y).abs() <= reach
}

/// The grid cell of a coordinate. A coordinate that is not a number lands in cell 0.
fn cell_of(coordinate: f32) -> i64 {
    (coordinate / CELL).floor() as i64
}

/// Drop the runs that draw an earlier run again on top of itself. Returns how many went.
///
/// One pass in the order the stream draws: a run is compared with the runs already kept near
/// it (a grid of cells, per text), so the earlier run is the one that stays and the work is
/// linear in the number of runs whatever the page holds.
///
/// The run that stays keeps its own place and style; it takes over the later run's links only
/// when it has none of its own (a link annotation over the text belongs to the word, not to
/// one of its copies).
pub(super) fn drop_overprinted_runs(spans: &mut Vec<TextSpan>) -> usize {
    let mut dropped = vec![false; spans.len()];
    let mut inherit: Vec<(usize, usize)> = Vec::new();
    {
        let mut kept: HashMap<(&str, i64, i64), Vec<usize>> = HashMap::new();
        for (i, span) in spans.iter().enumerate() {
            if span.text.trim().is_empty()
                || span.font_size.is_nan()
                || span.font_size < SIZE_FLOOR
                || !span.x.is_finite()
                || !span.y.is_finite()
            {
                continue;
            }
            let (cx, cy) = (cell_of(span.x), cell_of(span.y));
            // The earlier run's reach may be a little wider than this one's (sizes differ by
            // up to the slack), hence the margin.
            let radius = (reach_of(span) * (1.0 + 2.0 * OVERPRINT_SIZE_SLACK) / CELL).ceil() as i64;
            let mut original = None;
            'search: for dx in -radius..=radius {
                for dy in -radius..=radius {
                    let key = (
                        span.text.as_str(),
                        cx.saturating_add(dx),
                        cy.saturating_add(dy),
                    );
                    if let Some(list) = kept.get(&key) {
                        for &k in list.iter().rev().take(MAX_COMPARED) {
                            if is_overprint(&spans[k], span) {
                                original = Some(k);
                                break 'search;
                            }
                        }
                    }
                }
            }
            match original {
                Some(keeper) => {
                    dropped[i] = true;
                    if spans[keeper].links.is_empty() && !span.links.is_empty() {
                        inherit.push((keeper, i));
                    }
                }
                None => kept
                    .entry((span.text.as_str(), cx, cy))
                    .or_default()
                    .push(i),
            }
        }
    }

    let count = dropped.iter().filter(|&&d| d).count();
    if count == 0 {
        return 0;
    }
    for (keeper, source) in inherit {
        spans[keeper].links = spans[source].links.clone();
    }
    let mut index = 0;
    spans.retain(|_| {
        index += 1;
        !dropped[index - 1]
    });
    log::debug!("{count} run(s) drawn again on top of themselves dropped");
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, x: f32, y: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            links: Vec::new(),
            x,
            y,
            width: text.len() as f32 * 5.0,
            width_measured: true,
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

    fn texts(spans: &[TextSpan]) -> Vec<&str> {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn a_run_drawn_twice_in_the_same_place_is_read_once() {
        let mut spans = vec![
            span("Collection", 88.8, 652.7),
            span("Collection", 88.8, 652.7),
        ];
        assert_eq!(drop_overprinted_runs(&mut spans), 1);
        assert_eq!(texts(&spans), ["Collection"]);
    }

    #[test]
    fn a_shadow_a_fraction_of_a_point_away_is_the_same_drawing() {
        let mut spans = vec![span("Violet", 100.0, 500.0), span("Violet", 100.9, 500.9)];
        assert_eq!(drop_overprinted_runs(&mut spans), 1);
        assert_eq!(texts(&spans), ["Violet"]);
    }

    #[test]
    fn the_earlier_run_stays_whichever_is_further_left() {
        let mut first = span("Notice:", 100.5, 500.0);
        first.is_bold = true;
        let mut spans = vec![first, span("Notice:", 100.0, 500.0)];
        drop_overprinted_runs(&mut spans);
        assert_eq!(spans.len(), 1);
        assert!(
            spans[0].is_bold,
            "the run the stream draws first is the one kept"
        );
    }

    #[test]
    fn equal_words_further_apart_are_different_marks() {
        // A column of identical labels, one line apart; and the same word in two columns.
        let mut spans = vec![
            span("0", 100.0, 500.0),
            span("0", 100.0, 488.0),
            span("0", 300.0, 500.0),
        ];
        assert_eq!(drop_overprinted_runs(&mut spans), 0);
        assert_eq!(spans.len(), 3);
    }

    #[test]
    fn a_shadow_further_than_a_tenth_of_an_em_is_not_merged() {
        let mut spans = vec![span("Total", 100.0, 500.0), span("Total", 101.2, 500.0)];
        assert_eq!(drop_overprinted_runs(&mut spans), 0);
    }

    #[test]
    fn a_different_size_font_or_direction_is_a_different_drawing() {
        let mut bigger = span("Note", 100.0, 500.0);
        bigger.font_size = 12.0;
        let mut other_font = span("Note", 100.0, 500.0);
        other_font.font_name = "Times".to_string();
        let mut turned = span("Note", 100.0, 500.0);
        turned.direction = [0.0, 1.0];
        let mut spans = vec![span("Note", 100.0, 500.0), bigger, other_font, turned];
        assert_eq!(drop_overprinted_runs(&mut spans), 0);
    }

    #[test]
    fn blank_runs_are_left_alone() {
        let mut spans = vec![span(" ", 100.0, 500.0), span(" ", 100.0, 500.0)];
        assert_eq!(drop_overprinted_runs(&mut spans), 0);
    }

    /// A page built to be slow: the same label stacked at one `x`, a line apart, sixty thousand
    /// times, and the same run drawn many times over at each of a few places. Comparing every
    /// run with every other at its `x` would be quadratic — the input contract is a bounded time.
    #[test]
    fn a_page_of_stacked_identical_runs_is_not_quadratic() {
        let started = std::time::Instant::now();
        let mut spans: Vec<TextSpan> = (0..60_000)
            .map(|i| span(".", 100.0, 700.0 - i as f32 * 12.0))
            .collect();
        spans.extend((0..60_000).map(|i| span("x", 300.0, 700.0 - (i % 4) as f32 * 12.0)));
        let dropped = drop_overprinted_runs(&mut spans);
        assert_eq!(
            dropped,
            60_000 - 4,
            "all but one copy per place of the second kind"
        );
        assert_eq!(spans.len(), 60_000 + 4);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "took {:?}",
            started.elapsed()
        );
    }

    /// Runs that differ only in the font they name are not copies of each other: a crowd of them
    /// in one cell is all kept, and costs a bounded number of comparisons each.
    #[test]
    fn a_crowd_of_fonts_in_one_place_is_kept_and_cheap() {
        let started = std::time::Instant::now();
        let mut spans: Vec<TextSpan> = (0..5_000)
            .map(|i| {
                let mut s = span("A", 100.0, 500.0);
                s.font_name = format!("Font{i}");
                s
            })
            .collect();
        assert_eq!(drop_overprinted_runs(&mut spans), 0);
        assert_eq!(spans.len(), 5_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }

    #[test]
    fn three_copies_leave_the_first() {
        let mut spans = vec![
            span("Bold", 10.0, 10.0),
            span("Bold", 10.0, 10.0),
            span("Bold", 10.3, 10.0),
            span("after", 60.0, 10.0),
        ];
        assert_eq!(drop_overprinted_runs(&mut spans), 2);
        assert_eq!(texts(&spans), ["Bold", "after"]);
    }
}
