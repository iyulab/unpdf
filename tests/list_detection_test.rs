//! End-to-end coverage for list detection through the real `PdfParser` ->
//! markdown pipeline (cycle-35).
//!
//! `src/parser/layout.rs`'s own unit tests already prove `detect_list_marker`
//! and `LayoutAnalyzer::finish_block` classify a line/block correctly in
//! isolation. What they cannot prove is that the parser actually wires a
//! `BlockType::ListItem` block through to a real `Paragraph` carrying
//! `ListInfo`, and that the markdown renderer turns *that* into real
//! per-item structure — before this cycle, `BlockType::ListItem` was
//! constructed nowhere in the parser, so the four lines below would have
//! merged into one `should_break_block`-undetected paragraph at this
//! uniform, ordinary-body-text line spacing (15pt): a single flowing line
//! joined by spaces, not four separate list-item lines. A `.contains()`
//! check on the marker text alone can't tell the two apart — the source
//! PDF's literal text already starts with `"- "`/`"1. "`, so *that* substring
//! survives even in the unfixed merged-paragraph fallback. The discriminator
//! that only a real fix produces is each item landing on its own line.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

#[test]
fn bullet_and_numbered_items_render_as_separate_markdown_list_lines() {
    let doc = PdfParser::from_bytes(&common::list_items_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    let text = to_markdown(&doc, &RenderOptions::default()).expect("markdown render");
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();

    assert!(
        lines.contains(&"- First bullet item"),
        "expected the first bullet on its own CommonMark unordered-list line, got: {text:?}"
    );
    assert!(
        lines.contains(&"- Second bullet item"),
        "expected the second bullet on its own CommonMark unordered-list line, got: {text:?}"
    );
    assert!(
        lines.contains(&"1. First numbered item"),
        "expected the first item on its own CommonMark ordered-list line \
         reading the printed number, got: {text:?}"
    );
    assert!(
        lines.contains(&"2. Second numbered item"),
        "expected the second item on its own CommonMark ordered-list line \
         reading the printed number, got: {text:?}"
    );

    // The unfixed fallback (all four lines merged into one paragraph block)
    // would join them with a single space on one line instead.
    assert!(
        !text.contains("bullet item - Second") && !text.contains("bullet item 1. First"),
        "list items must not be merged into one flowing paragraph line: {text:?}"
    );
}
