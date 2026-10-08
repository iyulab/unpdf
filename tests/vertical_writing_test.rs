//! Composite fonts in vertical writing mode.
//!
//! Under a vertical CMap (a `-V` name, `Identity-V`, or `/WMode 1`) glyphs advance down the
//! page by the CIDFont's `/W2` displacement, or its `/DW2` default `[880 -1000]`, and each
//! glyph's origin sits at a position vector from the text position (ISO 32000-1 §9.7.4.3).
//! Without those advances several strings of one text object land on one spot. The fixtures
//! are written by `tests/fixtures/make_vertical_fixtures.py`: three single-glyph strings at
//! 20 pt, starting at (300, 180).

use unpdf::parser::backend::{PdfBackend, RawBackend, ResourceScope};
use unpdf::parser::{LayoutAnalyzer, TextSpan};

const DEFAULT: &[u8] = include_bytes!("fixtures/vertical-default.pdf");
const W2: &[u8] = include_bytes!("fixtures/vertical-w2.pdf");
const W2_RANGE: &[u8] = include_bytes!("fixtures/vertical-w2-range.pdf");
const DW2: &[u8] = include_bytes!("fixtures/vertical-dw2.pdf");
const ORIGIN: &[u8] = include_bytes!("fixtures/vertical-origin.pdf");
const TJ: &[u8] = include_bytes!("fixtures/vertical-tj.pdf");
const SPACING: &[u8] = include_bytes!("fixtures/vertical-spacing.pdf");
const MALFORMED: &[u8] = include_bytes!("fixtures/vertical-malformed-w2.pdf");
const IDENTITY: &[u8] = include_bytes!("fixtures/vertical-identity.pdf");
const EMBEDDED: &[u8] = include_bytes!("fixtures/vertical-embedded.pdf");

const GLYPHS: [&str; 3] = ["\u{3042}", "\u{3044}", "\u{3046}"];

fn spans(pdf: &[u8]) -> Vec<TextSpan> {
    let backend = RawBackend::load_bytes(pdf).expect("fixture parses");
    LayoutAnalyzer::new(&backend)
        .extract_page_spans(1)
        .expect("spans")
}

/// The baseline-origin y of each glyph's span, in reading order, asserting that every
/// glyph was extracted as its own span.
fn ys(pdf: &[u8]) -> Vec<f32> {
    let spans = spans(pdf);
    GLYPHS
        .iter()
        .map(|g| {
            spans
                .iter()
                .find(|s| s.text == *g)
                .unwrap_or_else(|| panic!("no {g:?} span in {spans:?}"))
                .y
        })
        .collect()
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.01,
        "expected {expected}, got {actual}"
    );
}

/// Successive drops down the column.
fn drops(pdf: &[u8]) -> Vec<f32> {
    let ys = ys(pdf);
    ys.windows(2).map(|w| w[0] - w[1]).collect()
}

#[test]
fn strings_in_one_text_object_stack_down_by_the_default_vertical_advance() {
    // /DW2 defaults to [880 -1000]: a full em down per glyph.
    let drops = drops(DEFAULT);
    assert_close(drops[0], 20.0);
    assert_close(drops[1], 20.0);
}

#[test]
fn the_glyph_origin_sits_at_the_position_vector_from_the_text_position() {
    // Default vector (w0 / 2, 880): the first glyph's horizontal-mode origin is 10 pt left of
    // and 17.6 pt below the text position (300, 180).
    let spans = spans(DEFAULT);
    assert_close(spans[0].x, 290.0);
    assert_close(spans[0].y, 162.4);
    // A /W2 vector of (250, 700) puts it 5 pt left and 14 pt below.
    let spans = spans_of_origin();
    assert_close(spans[0].x, 295.0);
    assert_close(spans[0].y, 166.0);
}

fn spans_of_origin() -> Vec<TextSpan> {
    spans(ORIGIN)
}

#[test]
fn w2_gives_a_glyph_its_own_vertical_advance_and_dw2_covers_the_rest() {
    // あ -500, い the default -1000, う -1500: drops of 10 then 20.
    let drops = drops(W2);
    assert_close(drops[0], 10.0);
    assert_close(drops[1], 20.0);
}

#[test]
fn w2_ranges_apply_to_every_cid_in_them() {
    let drops = drops(W2_RANGE);
    assert_close(drops[0], 8.0);
    assert_close(drops[1], 8.0);
}

#[test]
fn dw2_replaces_the_default_vertical_advance_and_origin() {
    let drops = drops(DW2);
    assert_close(drops[0], 12.0);
    assert_close(drops[1], 12.0);
    // vy = 800: 16 pt below the text position.
    assert_close(spans(DW2)[0].y, 164.0);
}

#[test]
fn a_tj_adjustment_moves_the_text_position_vertically() {
    // -500 raises the next glyph by half an em (a positive number lowers it): after あ the
    // position is 20 - 10 lower, and い's TJ opens with 1000, another full em down.
    let drops = drops(TJ);
    assert_close(drops[0], 30.0);
    assert_close(drops[1], 20.0);
}

#[test]
fn character_spacing_applies_to_the_vertical_advance_and_scale_does_not() {
    // ty = w1 * Tfs + Tc + Tw, with no horizontal scaling: -20 + 5 per glyph (Tw is for the
    // single-byte space only).
    let drops = drops(SPACING);
    assert_close(drops[0], 15.0);
    assert_close(drops[1], 15.0);
}

#[test]
fn a_malformed_w2_neither_panics_nor_costs_the_text() {
    let text = unpdf::parse_bytes(MALFORMED).expect("parses").plain_text();
    for g in GLYPHS {
        assert!(text.contains(g), "expected {g:?} in {text:?}");
    }
}

#[test]
fn the_advances_a_backend_reports_carry_the_vertical_displacement() {
    let backend = RawBackend::load_bytes(IDENTITY).expect("fixture parses");
    let scope = ResourceScope::page(backend.pages()[&1]);
    let advances = backend
        .glyph_advances(scope, b"F1", &[0, 1, 0, 2])
        .expect("an Identity-V font is measured");
    let w1y: Vec<f32> = advances
        .iter()
        .map(|a| a.vertical.expect("vertical").advance)
        .collect();
    assert_eq!(w1y, [-700.0, -1000.0]);
}

#[test]
fn an_embedded_cmap_with_wmode_1_stacks_the_glyphs() {
    let drops = drops(EMBEDDED);
    assert_close(drops[0], 20.0);
    assert_close(drops[1], 20.0);
}

#[test]
fn a_column_reads_top_to_bottom() {
    for pdf in [DEFAULT, W2, DW2, EMBEDDED] {
        let doc = unpdf::parse_bytes(pdf).expect("parses");
        let text = doc.plain_text();
        let positions: Vec<usize> = GLYPHS
            .iter()
            .map(|g| {
                text.find(g)
                    .unwrap_or_else(|| panic!("no {g:?} in {text:?}"))
            })
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text:?}");
    }
}
