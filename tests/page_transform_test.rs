//! Text positions follow the PDF coordinate transform exactly as the spec defines it.
//!
//! A nested `cm` is concatenated as `cm × CTM`. Getting the order backwards is
//! invisible on files that never leave the identity CTM, and places every run far
//! off the page on files that don't — browser-printed PDFs among them. Off-page
//! runs are then misjudged by anything that reasons about page position; a run
//! holding only a number reads as a page number in the margin and is dropped.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

#[test]
fn text_under_a_nested_cm_keeps_its_numbers() {
    let doc = PdfParser::from_bytes(&common::browser_printed_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic browser-printed PDF should parse");
    let md = to_markdown(&doc, &RenderOptions::default()).expect("markdown renders");

    assert!(md.contains("Volume"), "text run lost: {md:?}");
    assert!(
        md.contains("396"),
        "a numeric run drawn mid-page must not be treated as a margin page number: {md:?}"
    );
}
