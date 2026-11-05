//! Text drawn again on top of itself — a producer's way to bold a word, or give it a shadow —
//! is one word on the page and one word in the output.

mod common;

use common::page_pdf;

const LETTER: &str = "/MediaBox[0 0 612 792]";

fn markdown(content: &str) -> String {
    let doc = unpdf::parse_bytes(&page_pdf("", LETTER, content.as_bytes())).expect("parses");
    unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default()).expect("renders")
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The same string shown twice at the same place: one `Collection`, not `CollectionCollection`.
#[test]
fn a_word_shown_twice_in_the_same_place_is_read_once() {
    let md = markdown(
        "BT /F1 10 Tf 72 700 Td (Collection of data) Tj ET\n\
         BT /F1 10 Tf 72 700 Td (Collection of data) Tj ET\n",
    );
    assert_eq!(count(&md, "Collection"), 1, "{md}");
    assert!(md.contains("Collection of data"), "{md}");
}

/// A shadow a fraction of a point off is the same drawing.
#[test]
fn a_shadow_under_a_word_is_not_read_as_a_second_word() {
    let md = markdown(
        "BT /F1 11 Tf 72 700 Td (Violet) Tj ET\n\
         BT /F1 11 Tf 72.6 700.6 Td (Violet) Tj ET\n",
    );
    assert_eq!(count(&md, "Violet"), 1, "{md}");
}

/// The same word on the next line, and the same word again further along the line, are
/// different marks: nothing is lost.
#[test]
fn the_same_word_elsewhere_is_kept() {
    let md = markdown(
        "BT /F1 11 Tf 72 700 Td (Total) Tj ET\n\
         BT /F1 11 Tf 72 686 Td (Total) Tj ET\n\
         BT /F1 11 Tf 300 700 Td (Total) Tj ET\n",
    );
    assert_eq!(count(&md, "Total"), 3, "{md}");
}
