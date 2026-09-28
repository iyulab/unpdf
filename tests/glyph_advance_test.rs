//! Text positions advance by the glyph widths the font declares.
//!
//! Showing text moves the text position by each glyph's advance (ISO 32000-1
//! §9.4.4). A producer that draws one glyph per operator places each glyph at the
//! previous one's advance, so the font's `/Widths` are the only correct measure of
//! where one glyph ends and the next begins — a per-character guess is off by a
//! fraction of an em for wide and narrow glyphs, which is enough to split a word.

mod common;

use unpdf::render::{to_text, RenderOptions};
use unpdf::PdfParser;

fn text_of(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf)
        .and_then(|p| p.parse())
        .expect("synthetic PDF should parse");
    to_text(&doc, &RenderOptions::default()).expect("text renders")
}

#[test]
fn a_word_drawn_one_glyph_at_a_time_stays_whole() {
    // `m` (0.889 em) is far wider than an average glyph: a guessed advance leaves a
    // gap after it that reads as a word break.
    let text = text_of(&common::glyph_per_operator_pdf(
        "carbon concentrating mechanisms",
        14.0,
        true,
    ));
    assert!(
        text.contains("carbon concentrating mechanisms"),
        "the line must read as three whole words, got {text:?}"
    );
}

#[test]
fn a_word_gap_drawn_as_a_move_still_separates_words() {
    // No space glyph: the gap between words exists only as a longer `Td`. Joining
    // glyphs that abut must not also join glyphs a word space apart.
    let text = text_of(&common::glyph_per_operator_pdf(
        "carbon concentrating mechanisms",
        14.0,
        false,
    ));
    assert!(
        text.contains("carbon concentrating mechanisms"),
        "the word gaps must survive, got {text:?}"
    );
}
