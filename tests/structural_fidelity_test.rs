//! Structural-fidelity benchmark (1st increment): a small domain fixture
//! corpus scored against hand-authored expectations via
//! `common::fidelity::score`. This is the regression floor for extraction
//! *quality* (block-type classification, text recovery, table-cell
//! accuracy) that unit tests don't cover — they assert one field at a time
//! and don't notice a same-shape-different-content drift the way a
//! continuous score does.
//!
//! Run with `cargo test --test structural_fidelity_test -- --nocapture` to
//! see the per-fixture score report.
//!
//! Scope note: this is the 1st increment of the benchmark-infrastructure
//! phase (`claudedocs/unpdf/cycle-logs/ROADMAP.md`). LLM-as-judge scoring
//! and a public cross-tool comparison page are later increments, not this
//! one — this increment only needs ground truth this repo can author and
//! check into version control, which real-world PDF fixtures (undetermined
//! license, `test-files/` gitignored) are not.

mod common;

use common::fidelity::{score, ExpectedBlock, FidelityScore};
use unpdf::PdfParser;

fn parse(bytes: &[u8]) -> unpdf::model::Document {
    PdfParser::from_bytes(bytes)
        .and_then(|p| p.parse())
        .expect("fixture PDF should parse")
}

/// Fails the test with a readable breakdown if `actual` is below `min` on
/// any component — a bare `assert!(overall > x)` would hide which axis
/// regressed.
fn assert_min(fixture: &str, actual: FidelityScore, min: FidelityScore) {
    println!(
        "[fidelity] {fixture:<24} type_sequence={:.2} text_similarity={:.2} table_cell_accuracy={:.2} overall={:.2}",
        actual.type_sequence,
        actual.text_similarity,
        actual.table_cell_accuracy,
        actual.overall(),
    );
    assert!(
        actual.type_sequence >= min.type_sequence,
        "{fixture}: type_sequence {:.2} < floor {:.2}",
        actual.type_sequence,
        min.type_sequence
    );
    assert!(
        actual.text_similarity >= min.text_similarity,
        "{fixture}: text_similarity {:.2} < floor {:.2}",
        actual.text_similarity,
        min.text_similarity
    );
    assert!(
        actual.table_cell_accuracy >= min.table_cell_accuracy,
        "{fixture}: table_cell_accuracy {:.2} < floor {:.2}",
        actual.table_cell_accuracy,
        min.table_cell_accuracy
    );
}

const PERFECT: FidelityScore = FidelityScore {
    type_sequence: 1.0,
    text_similarity: 1.0,
    table_cell_accuracy: 1.0,
};

#[test]
fn single_paragraph_fixture_scores_perfectly() {
    let doc = parse(&common::text_pdf());
    let expected = vec![ExpectedBlock::Paragraph {
        text: "Hello World",
    }];
    assert_min("text_pdf", score(&doc, &expected), PERFECT);
}

#[test]
fn bordered_table_fixture_scores_perfectly() {
    let doc = parse(&common::bordered_table_pdf());
    let expected = vec![ExpectedBlock::Table {
        rows: vec![vec!["Name", "Age"], vec!["Alice", "30"]],
    }];
    assert_min("bordered_table_pdf", score(&doc, &expected), PERFECT);
}

#[test]
fn heading_and_paragraph_fixture_scores_perfectly() {
    let doc = parse(&common::heading_paragraph_pdf());
    let expected = vec![
        ExpectedBlock::Heading {
            level: 1,
            text: "Chapter One",
        },
        ExpectedBlock::Paragraph {
            text: "This is the first paragraph of the chapter. It continues on a second line of body text.",
        },
    ];
    assert_min("heading_paragraph_pdf", score(&doc, &expected), PERFECT);
}

/// Reading order must follow XY-Cut column grouping (left column, top to
/// bottom, then right column), not content-stream emission order or a
/// naive y-descending sort — see `common::two_column_pdf`'s doc comment for
/// why the fixture is built adversarially against those two shortcuts.
#[test]
fn two_column_fixture_preserves_left_to_right_reading_order() {
    let doc = parse(&common::two_column_pdf());
    let expected = vec![
        ExpectedBlock::Paragraph {
            text: "Left column paragraph text here.",
        },
        ExpectedBlock::Paragraph {
            text: "Right column paragraph text here.",
        },
    ];
    assert_min("two_column_pdf", score(&doc, &expected), PERFECT);
}

/// Predictive-CMap CJK decoding (`parser::cmap_table`) — no embedded font
/// program, no `ToUnicode`, only `CIDSystemInfo`. See `common::cjk_pdf`'s
/// doc comment for how the two CIDs were verified against `lookup_cid`
/// directly rather than guessed.
#[test]
fn cjk_fixture_scores_perfectly() {
    let doc = parse(&common::cjk_pdf());
    let expected = vec![ExpectedBlock::Paragraph { text: "가방" }];
    assert_min("cjk_pdf", score(&doc, &expected), PERFECT);
}

/// A page with nothing (`common::blank_pdf`) scored against an empty
/// expectation must not be penalized by the "not applicable" components —
/// guards `FidelityScore`'s own semantics, not the parser.
#[test]
fn empty_page_against_empty_expectation_scores_perfectly() {
    let doc = parse(&common::blank_pdf());
    assert_min("blank_pdf", score(&doc, &[]), PERFECT);
}
