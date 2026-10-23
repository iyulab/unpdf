//! Subscripts and superscripts belong to the line whose baseline they sit on.
//!
//! A script is set smaller and shifted off the baseline — often by more than a
//! same-line tolerance allows — so grouping by baseline alone makes it a line of its
//! own. That line then lands between two body lines, becomes a paragraph of its own,
//! and for a superscript is emitted *before* the line it belongs to.

mod common;

use unpdf::render::{to_markdown, to_text, RenderOptions};
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

/// Lines set across the page above two columns are cut off for reading one band at a time,
/// and a superscript raised into the gap above its line has a baseline of its own -- so it
/// was a band of its own, and nothing could rejoin it to its line across that boundary.
#[test]
fn a_superscript_in_lines_set_over_two_columns_stays_on_its_line() {
    let doc = PdfParser::from_bytes(&common::scripted_band_over_columns_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    let text = to_text(&doc, &RenderOptions::default()).expect("text renders");
    assert!(
        text.contains("methane formed mol1, for growth")
            || text.contains("methane formed mol 1, for growth"),
        "superscript detached from its line: {text:?}"
    );
    let above = text.find("near 3 and 7 g of").expect("line above present");
    let own = text.find("methane formed mol").expect("its line present");
    let sup = text[above..]
        .find('1')
        .map(|i| i + above)
        .expect("superscript present");
    assert!(
        own < sup,
        "superscript emitted before its own line: {text:?}"
    );
    // The columns below still read one after the other.
    let left = text
        .find("Left column body line number 7")
        .expect("left column");
    let right = text
        .find("Right column body line number 0")
        .expect("right column");
    assert!(left < right, "columns interleaved: {text:?}");
}

/// A script folded into its line is still written as one.
#[test]
fn a_folded_script_keeps_its_script_style() {
    let doc = PdfParser::from_bytes(&common::scripted_lines_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    let md = to_markdown(&doc, &RenderOptions::default()).expect("markdown renders");
    assert!(md.contains("precursor to C<sub>4</sub>"), "{md:?}");
    assert!(md.contains(r"own right.<sup>\[35\]</sup>"), "{md:?}");
}
