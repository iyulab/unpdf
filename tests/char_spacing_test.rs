//! A word gap is the room between two glyphs, however a producer makes it — a `TJ`
//! adjustment, or character spacing (`Tc`) that the adjustments inside each word take back.
//!
//! Every glyph of the font below is 500 units wide; text is set at `1 Tf` scaled to 10 pt by
//! the text matrix, so `Tc` is in ems.

mod common;

use common::{assemble, stream_object};

fn monospaced_font() -> Vec<u8> {
    let widths = vec!["500"; 95].join(" ");
    format!("<</Type/Font/Subtype/Type1/BaseFont/Mono/FirstChar 32/LastChar 126/Widths[{widths}]>>")
        .into_bytes()
}

fn text_of(line: &str) -> String {
    let content = format!("BT /F1 1 Tf 10 0 0 10 40 300 Tm {line} ET");
    let pdf = assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 400]/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content.as_bytes()),
        monospaced_font(),
    ]);
    unpdf::parse_bytes(&pdf).expect("parses").plain_text()
}

/// `.381 Tc` puts 0.381 em after every glyph; `375` takes it back inside each word. What is
/// left is the gap between words — and a string of two glyphs (`(es)`) holds a word boundary.
#[test]
fn character_spacing_that_the_adjustments_take_back_inside_words_separates_words() {
    let text =
        text_of(".381 Tc [(t)375(h)375(es)375(e)375(c)375(o)375(n)375(d)-10(o)375(n)375(e)] TJ");
    assert_eq!(text.trim(), "the second one");
}

/// Tracking — a little character spacing over a whole word — is not a word gap.
#[test]
fn light_tracking_does_not_space_a_word_out() {
    assert_eq!(text_of(".05 Tc (TITLE) Tj").trim(), "TITLE");
}

/// Two adjustments in a row are one gap.
#[test]
fn adjacent_adjustments_add_up_to_one_gap() {
    assert_eq!(text_of("[(ab)-70 -70(cd)] TJ").trim(), "ab cd");
}

/// A contents leader set with character spacing is a run of dots, not dots separated by
/// spaces.
#[test]
fn spaced_out_leader_dots_stay_one_run() {
    assert_eq!(text_of(".3 Tc (..........) Tj").trim(), "..........");
}

/// Letter-spaced type has wide room between every two letters; a word gap is room beyond it.
#[test]
fn letter_spaced_words_stay_words() {
    assert_eq!(text_of(".2 Tc [(THE)-333(PLAN)] TJ").trim(), "THE PLAN");
    assert_eq!(
        text_of(".2 Tc [(T)(H)(E)-333(P)(L)(A)(N)] TJ").trim(),
        "THE PLAN"
    );
}

/// Tight tracking pulls letters together; a word gap narrower than a fixed threshold is still
/// a gap against it.
#[test]
fn a_word_gap_in_tightly_tracked_type_is_a_gap() {
    assert_eq!(
        text_of("-.03 Tc [(ALICE)-140(JONES)] TJ").trim(),
        "ALICE JONES"
    );
}

/// A producer that draws the letters either side of a word gap as one string (`(ya)` — the `y`
/// of `ability`, the `a` of `and`) and opens them with a wide `Tc`: the string shows no letter
/// room but its `Tc`, the strings around it set their letters closer, and the gap is a space.
#[test]
fn a_wide_tc_on_a_lone_two_letter_string_is_a_word_gap_on_its_line() {
    let text = text_of("0 Tc (the abilit) Tj .27 Tc (ya) Tj 0 Tc (nd more) Tj");
    assert_eq!(text.trim(), "the ability and more");
}

/// A letter-spaced heading has the same wide `Tc` in every string of its line: it stands out
/// from nothing, and stays one word.
#[test]
fn a_letter_spaced_heading_drawn_in_pieces_stays_one_word() {
    assert_eq!(
        text_of(".246 Tc (DEFINI) Tj (TIONS) Tj").trim(),
        "DEFINITIONS"
    );
}

/// Beside tightly tracked text, a word set with ordinary letter spacing is not spaced out.
#[test]
fn ordinary_tracking_beside_tight_tracking_is_not_a_gap() {
    assert_eq!(
        text_of("-.05 Tc (tight text here ) Tj .08 Tc (word) Tj").trim(),
        "tight text here word"
    );
}

/// A heading in small capitals: a larger capital, then the rest of the word letter-spaced. The
/// capital alone shows no room between letters, and the word stays a word.
#[test]
fn small_capitals_after_a_larger_initial_stay_one_word() {
    let text = text_of("0 Tc (D) Tj 8 0 0 8 45 300 Tm .25 Tc (EFINITIONS) Tj");
    assert!(!text.contains("E F"), "{text}");
}

/// Character spacing of several ems is room between cells: `4 Tc` puts four ems after every
/// glyph, so the two glyphs of `(ab)` stand 40 pt apart and read as two pieces of text.
#[test]
fn character_spacing_of_several_ems_separates_what_it_spreads() {
    assert_eq!(text_of("4 Tc (ab) Tj").trim(), "a b");
    assert_eq!(text_of("4 Tc [(a)(b)] TJ").trim(), "a b");
    // The glyphs of one string and the strings of one array are each placed where the
    // spacing leaves them: three pieces in all, in order.
    assert_eq!(text_of("4 Tc [(ab)(c)] TJ").trim(), "a b c");
}

/// Tracked type — spacing up to an em or two — is letters of one word, however wide.
#[test]
fn character_spacing_of_an_em_or_two_keeps_letters_together() {
    assert_eq!(text_of(".5 Tc (ab) Tj").trim(), "ab");
    assert_eq!(text_of("2 Tc (ab) Tj").trim(), "ab");
}
