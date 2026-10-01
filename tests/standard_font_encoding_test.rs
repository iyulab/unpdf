//! A standard font without `/Encoding` speaks its built-in encoding.
//!
//! The 14 standard fonts may be named by `/BaseFont` alone (ISO 32000-1 §9.6.6):
//! the Latin faces then use StandardEncoding, and Symbol and ZapfDingbats each use
//! an encoding of their own. Reading their codes as Latin-1 turns a check mark into
//! `4` and a Greek alpha into `a`.

mod common;

use unpdf::render::{to_text, RenderOptions};
use unpdf::PdfParser;

fn text_of(runs: &[(&str, &str)]) -> String {
    let doc = PdfParser::from_bytes(&common::standard_fonts_line_pdf(runs))
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    to_text(&doc, &RenderOptions::default()).expect("text renders")
}

#[test]
fn zapf_dingbats_codes_read_as_the_dingbats_they_draw() {
    // `4` is the heavy check mark — drawn before a label the way forms mark a choice.
    let text = text_of(&[("ZapfDingbats", "4"), ("Helvetica", "Approved")]);
    assert!(
        text.contains("\u{2714}") && !text.contains('4'),
        "the check mark must come out as a check mark, got {text:?}"
    );
    assert!(
        text.contains("Approved"),
        "the label survives, got {text:?}"
    );
}

#[test]
fn symbol_codes_read_as_greek_and_math() {
    // `a b D` are alpha, beta, Delta; `\245` (0xA5) is infinity.
    let text = text_of(&[("Symbol", "abD \\245")]);
    assert!(
        text.contains("\u{03B1}\u{03B2}\u{0394} \u{221E}"),
        "Symbol must come out as the Greek and math glyphs it draws, got {text:?}"
    );
}

#[test]
fn a_latin_standard_font_uses_standard_encoding() {
    // StandardEncoding draws a right single quote at 0x27.
    let text = text_of(&[("Helvetica", "don't stop")]);
    assert!(
        text.contains("don\u{2019}t stop"),
        "the apostrophe is the glyph StandardEncoding draws, got {text:?}"
    );
}
