//! Composite fonts whose `/Encoding` is a CMap stream embedded in the file.
//!
//! The fixtures are written by `tests/fixtures/make_embedded_cmap_fixtures.py`; each one
//! shows Korean text through a non-embedded Adobe-Korea1 CIDFont with no `/ToUnicode`, so
//! the only way to the text is the encoding stream.

use unpdf::parser::backend::{PdfBackend, RawBackend, ResourceScope, TextSuppression};

const USECMAP_UNICODE: &[u8] = include_bytes!("fixtures/embedded-cmap-usecmap-unicode.pdf");
const USECMAP_LEGACY: &[u8] = include_bytes!("fixtures/embedded-cmap-usecmap-legacy.pdf");
const RANGES: &[u8] = include_bytes!("fixtures/embedded-cmap-ranges.pdf");
const CHAIN: &[u8] = include_bytes!("fixtures/embedded-cmap-chain.pdf");
const VERTICAL: &[u8] = include_bytes!("fixtures/embedded-cmap-vertical.pdf");
const LOOP: &[u8] = include_bytes!("fixtures/embedded-cmap-loop.pdf");
const GARBAGE: &[u8] = include_bytes!("fixtures/embedded-cmap-garbage.pdf");

fn assert_reads(pdf: &[u8], line: &str) {
    let doc = unpdf::parse_bytes(pdf).expect("fixture parses");
    let text = doc.plain_text();
    assert!(text.contains(line), "expected {line:?} in {text:?}");
    assert_eq!(
        doc.extraction_quality.suppressed_text_runs, 0,
        "no run may be discarded, got {text:?}"
    );
    assert!(doc.pages[0].unreadable_fonts.is_empty());
}

fn assert_unreadable(pdf: &[u8]) {
    let doc = unpdf::parse_bytes(pdf).expect("a bad encoding stream must not fail the parse");
    assert_eq!(doc.extraction_quality.suppressed_text_runs, 1);
    let fonts = &doc.pages[0].unreadable_fonts;
    assert_eq!(fonts.len(), 1, "{fonts:?}");
    assert_eq!(fonts[0].name, "HYSMyeongJo-Medium");
    assert_eq!(fonts[0].reason, TextSuppression::CompositeUnresolved);
    assert!(!doc.plain_text().contains('한'));
}

fn widths(pdf: &[u8], bytes: &[u8]) -> Option<Vec<f32>> {
    let backend = RawBackend::load_bytes(pdf).expect("fixture parses");
    let scope = ResourceScope::page(backend.pages()[&1]);
    backend
        .glyph_advances(scope, b"F1", bytes)
        .map(|advances| advances.iter().map(|a| a.width).collect())
}

#[test]
fn a_stream_that_only_builds_on_a_unicode_cmap_reads_the_unicode() {
    assert_reads(USECMAP_UNICODE, "한글 문서");
}

#[test]
fn a_stream_over_a_legacy_cmap_applies_its_own_ranges_on_top() {
    assert_reads(USECMAP_LEGACY, "한한 문서");
}

#[test]
fn a_fully_defined_cmap_splits_codes_by_its_code_space_and_maps_them_to_cids() {
    assert_reads(RANGES, "한글 문서A");
}

#[test]
fn a_use_cmap_stream_chain_is_followed() {
    assert_reads(CHAIN, "한글 문서A한");
}

#[test]
fn widths_are_keyed_by_the_cids_the_embedded_cmap_resolves() {
    // Under the code space of the fixture the space is the one-byte code 0x20 (CID 1,
    // width 333); the others take the default.
    let got = widths(RANGES, &[0x80, 0x01, 0x20, 0x80, 0x02, 0x41]).expect("measured");
    assert_eq!(got, [1000.0, 333.0, 1000.0, 1000.0]);
    let got = widths(USECMAP_UNICODE, &[0xD5, 0x5C, 0x00, 0x20]).expect("measured");
    assert_eq!(got, [1000.0, 333.0]);
}

#[test]
fn a_vertical_embedded_cmap_reads_but_is_not_measured_horizontally() {
    assert_reads(VERTICAL, "한글 문서A");
    assert_eq!(widths(VERTICAL, &[0x80, 0x01]), None);
}

#[test]
fn a_use_cmap_loop_is_reported_as_unreadable() {
    assert_unreadable(LOOP);
}

#[test]
fn a_stream_that_is_not_a_cmap_is_reported_as_unreadable() {
    assert_unreadable(GARBAGE);
}
