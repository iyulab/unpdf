//! Emphasis markers are written where CommonMark reads them as emphasis.
//!
//! A styled run that starts or ends in punctuation and abuts a letter or digit would put a
//! `*` between the two, which CommonMark does not take as a delimiter (§6.2) — the reader
//! sees stray asterisks. Both writers must place such punctuation outside the markers.

use unpdf::model::{Document, Page, Paragraph, TextRun};
use unpdf::render::streaming::{collect_content, StreamingRenderer};
use unpdf::render::{to_markdown, RenderOptions};

/// `n = 32, s = 48, and m = 8`, the way a TeX page sets it: the variables, and the comma
/// and space before `s`, in the math italic font.
fn math_line() -> Document {
    let mut para = Paragraph::new();
    para.add_run(TextRun::new("with "));
    para.add_run(TextRun::italic("n"));
    para.add_run(TextRun::new(" = 32"));
    para.add_run(TextRun::italic(", s"));
    para.add_run(TextRun::new(" = 48"));
    para.add_run(TextRun::italic(","));
    para.add_run(TextRun::new(" and "));
    para.add_run(TextRun::italic("m"));
    para.add_run(TextRun::new(" = 8."));
    let mut page = Page::letter(1);
    page.add_paragraph(para);
    let mut doc = Document::new();
    doc.add_page(page);
    doc
}

const EXPECTED: &str = "with *n* = 32, *s* = 48, and *m* = 8.";

#[test]
fn the_batch_writer_keeps_punctuation_outside_the_markers() {
    let md = to_markdown(&math_line(), &RenderOptions::default()).unwrap();
    assert!(md.contains(EXPECTED), "{md}");
}

#[test]
fn the_streaming_writer_keeps_punctuation_outside_the_markers() {
    let doc = math_line();
    let md = collect_content(StreamingRenderer::new(&doc, RenderOptions::default()));
    assert!(md.contains(EXPECTED), "{md}");
}
