//! Subscripts and superscripts belong to the line whose baseline they sit on.
//!
//! A script is set smaller and shifted off the baseline — often by more than a
//! same-line tolerance allows — so grouping by baseline alone makes it a line of its
//! own. That line then lands between two body lines, becomes a paragraph of its own,
//! and for a superscript is emitted *before* the line it belongs to.

mod common;

use unpdf::render::{to_text, RenderOptions};
use unpdf::PdfParser;

fn text() -> String {
    let doc = PdfParser::from_bytes(&common::scripted_lines_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    to_text(&doc, &RenderOptions::default()).expect("text renders")
}

#[test]
fn a_subscript_stays_on_its_line() {
    let text = text();
    assert!(
        text.contains("precursor to C4"),
        "subscript detached: {text:?}"
    );
}

#[test]
fn a_superscript_stays_on_its_line_and_in_order() {
    let text = text();
    assert!(
        text.contains("own right.[35]"),
        "superscript detached: {text:?}"
    );
    let sup = text.find("[35]").expect("superscript present");
    let line = text.find("and a useful").expect("its line present");
    assert!(
        line < sup,
        "superscript emitted before its own line: {text:?}"
    );
}
