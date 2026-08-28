//! Structural-fidelity scoring: compares an extracted [`Document`]'s block
//! structure against a small, hand-authored expectation.
//!
//! This is deliberately not a per-field unit assertion (`assert_eq!` on one
//! table's cells) — it is a continuous score meant to catch *regressions in
//! overall extraction quality* across a small fixture corpus, the same role
//! a benchmark suite plays for larger, real-world document sets. Component
//! scores are exposed separately (block-type sequence, matched-text
//! similarity, table-cell accuracy) so a regression in one axis doesn't hide
//! behind an average.

use unpdf::model::{Block, Document};

/// A minimal, hand-authored expectation for one page's block structure.
/// Mirrors the [`Block`] discriminants most affected by layout/table-detection
/// changes — the ones this metric exists to guard.
#[derive(Debug, Clone)]
pub enum ExpectedBlock {
    Heading { level: u8, text: &'static str },
    Paragraph { text: &'static str },
    Table { rows: Vec<Vec<&'static str>> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Heading,
    Paragraph,
    Table,
    Other,
}

fn actual_kind(b: &Block) -> Kind {
    match b {
        Block::Paragraph(p) if p.is_heading() => Kind::Heading,
        Block::Paragraph(_) => Kind::Paragraph,
        Block::Table(_) => Kind::Table,
        _ => Kind::Other,
    }
}

fn expected_kind(e: &ExpectedBlock) -> Kind {
    match e {
        ExpectedBlock::Heading { .. } => Kind::Heading,
        ExpectedBlock::Paragraph { .. } => Kind::Paragraph,
        ExpectedBlock::Table { .. } => Kind::Table,
    }
}

/// Per-fixture structural fidelity score. Each component is in `[0, 1]`;
/// a component with nothing to measure (e.g. no tables expected) reports
/// `1.0` — "not applicable" must not drag the average down.
#[derive(Debug, Clone, Copy)]
pub struct FidelityScore {
    /// F1 of the LCS block-type alignment between actual and expected.
    pub type_sequence: f32,
    /// Mean normalized text similarity across LCS-matched Heading/Paragraph pairs.
    pub text_similarity: f32,
    /// Mean cell-match ratio across LCS-matched Table pairs.
    pub table_cell_accuracy: f32,
}

impl FidelityScore {
    pub fn overall(&self) -> f32 {
        (self.type_sequence + self.text_similarity + self.table_cell_accuracy) / 3.0
    }
}

/// Score `actual` (the parsed document) against `expected` (flattened across
/// all pages, in document order — the fixtures this metric targets are
/// single-page).
pub fn score(actual: &Document, expected: &[ExpectedBlock]) -> FidelityScore {
    let actual_blocks: Vec<&Block> = actual.pages.iter().flat_map(|p| &p.elements).collect();
    let matches = lcs_matches(&actual_blocks, expected);

    let type_sequence = f1(matches.len(), actual_blocks.len(), expected.len());

    let mut text_scores = Vec::new();
    let mut table_scores = Vec::new();
    for (ai, ei) in &matches {
        match (&actual_blocks[*ai], &expected[*ei]) {
            (Block::Paragraph(p), ExpectedBlock::Heading { text, .. })
            | (Block::Paragraph(p), ExpectedBlock::Paragraph { text }) => {
                text_scores.push(text_similarity(&p.plain_text(), text));
            }
            (Block::Table(t), ExpectedBlock::Table { rows }) => {
                table_scores.push(table_cell_accuracy(t, rows));
            }
            _ => {}
        }
    }

    FidelityScore {
        type_sequence,
        text_similarity: mean_or_perfect(&text_scores),
        table_cell_accuracy: mean_or_perfect(&table_scores),
    }
}

fn mean_or_perfect(scores: &[f32]) -> f32 {
    if scores.is_empty() {
        1.0
    } else {
        scores.iter().sum::<f32>() / scores.len() as f32
    }
}

fn f1(matched: usize, actual_len: usize, expected_len: usize) -> f32 {
    if actual_len == 0 && expected_len == 0 {
        return 1.0;
    }
    if matched == 0 {
        return 0.0;
    }
    let precision = matched as f32 / actual_len as f32;
    let recall = matched as f32 / expected_len as f32;
    2.0 * precision * recall / (precision + recall)
}

/// Longest-common-subsequence alignment by block *kind* only — returns
/// matched `(actual_index, expected_index)` pairs in order. Order-preserving
/// (unlike a bag-of-kinds match), so a table detected out of place doesn't
/// score as a false positive for a table expected elsewhere on the page.
fn lcs_matches(actual: &[&Block], expected: &[ExpectedBlock]) -> Vec<(usize, usize)> {
    let n = actual.len();
    let m = expected.len();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if actual_kind(actual[i - 1]) == expected_kind(&expected[j - 1]) {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }

    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if actual_kind(actual[i - 1]) == expected_kind(&expected[j - 1])
            && dp[i][j] == dp[i - 1][j - 1] + 1
        {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

fn text_similarity(actual: &str, expected: &str) -> f32 {
    let actual = actual.trim();
    let expected = expected.trim();
    if actual.is_empty() && expected.is_empty() {
        return 1.0;
    }
    let dist = levenshtein(actual, expected) as f32;
    let max_len = actual.chars().count().max(expected.chars().count()) as f32;
    if max_len == 0.0 {
        1.0
    } else {
        (1.0 - dist / max_len).max(0.0)
    }
}

/// Cell-match ratio: exact-text matches over the union of the actual and
/// expected cell grids (mismatched dimensions count the extra/missing cells
/// as misses rather than being ignored).
fn table_cell_accuracy(actual: &unpdf::model::Table, expected_rows: &[Vec<&str>]) -> f32 {
    let mut total = 0usize;
    let mut matched = 0usize;

    let row_count = actual.rows.len().max(expected_rows.len());
    for r in 0..row_count {
        let actual_row = actual.rows.get(r);
        let expected_row = expected_rows.get(r);
        let col_count = actual_row
            .map(|row| row.cells.len())
            .unwrap_or(0)
            .max(expected_row.map(|row| row.len()).unwrap_or(0));
        for c in 0..col_count {
            total += 1;
            let actual_cell = actual_row
                .and_then(|row| row.cells.get(c))
                .map(|cell| cell.plain_text());
            let expected_cell = expected_row
                .and_then(|row| row.get(c))
                .map(|s| s.to_string());
            if actual_cell.as_deref().map(str::trim) == expected_cell.as_deref().map(str::trim) {
                matched += 1;
            }
        }
    }

    if total == 0 {
        1.0
    } else {
        matched as f32 / total as f32
    }
}

/// Classic O(n*m) edit distance — no new dependency for a metric that only
/// ever runs over short fixture strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];

    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_identical_is_zero() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn levenshtein_counts_substitutions() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn f1_perfect_match() {
        assert_eq!(f1(2, 2, 2), 1.0);
    }

    #[test]
    fn f1_no_match_is_zero() {
        assert_eq!(f1(0, 2, 2), 0.0);
    }

    #[test]
    fn f1_both_empty_is_perfect() {
        assert_eq!(f1(0, 0, 0), 1.0);
    }
}
