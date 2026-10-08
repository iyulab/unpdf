//! A line that opens with a negative number is text, not a list item.
//!
//! Statistical tables are often drawn as plain text lines, so a line can start with a
//! value such as `-0.2`. A dash glued to a digit is the number's sign: read as a bullet,
//! the sign was dropped (`• 0.2` in text, `- 0.2` in Markdown). A dash followed by a
//! space stays a list marker, whatever follows it, and so does a dash glued to a word.
//! The fixture is written by `tests/fixtures/make_signed_number_fixtures.py`.

use unpdf::render::{to_markdown, to_text, RenderOptions};

const PDF: &[u8] = include_bytes!("fixtures/signed-number-lines.pdf");

/// Lines that open with a signed number: they must come back exactly, sign included.
const SIGNED: &[&str] = &[
    "-0.2 4.5",
    "-2.9 -4.5 -2.4",
    "\u{2212}0.8 3.1",
    "\u{2013}0.7 1.2",
    "-.5",
    "-12",
];

fn render() -> (String, String) {
    let doc = unpdf::parse_bytes(PDF).expect("fixture parses");
    let text = to_text(&doc, &RenderOptions::default()).expect("text render");
    let markdown = to_markdown(&doc, &RenderOptions::default()).expect("markdown render");
    (text, markdown)
}

fn lines(s: &str) -> Vec<&str> {
    s.lines().map(str::trim_end).collect()
}

#[test]
fn a_line_opening_with_a_negative_number_keeps_its_sign_in_text() {
    let (text, _) = render();
    let lines = lines(&text);
    for signed in SIGNED {
        assert!(
            lines.contains(signed),
            "expected the line {signed:?} as it is printed, got: {text:?}"
        );
    }
}

#[test]
fn a_line_opening_with_a_negative_number_keeps_its_sign_in_markdown() {
    let (_, markdown) = render();
    let lines = lines(&markdown);
    for signed in SIGNED {
        assert!(
            lines.contains(signed),
            "expected the line {signed:?} as it is printed, got: {markdown:?}"
        );
    }
}

#[test]
fn a_dash_followed_by_a_space_or_glued_to_a_word_stays_a_list_marker() {
    let (text, markdown) = render();
    let text_lines = lines(&text);
    let markdown_lines = lines(&markdown);
    // `- 2023.1.12일 …`: a space after the dash makes it a marker, even before a number.
    // `– 0.3 하락`: an en dash and a space before a number reads the same way — the page
    // gives nothing to tell it from a bullet. `-외화 …`: a dash glued to a word is a bullet
    // drawn without a gap; a sign only ever stands before a number.
    for item in [
        "2023.1.12일 실시한 점검 결과를 반영하였다.",
        "외화 유동성 점검",
        "0.3 하락",
    ] {
        let bullet = format!("\u{2022} {item}");
        assert!(
            text_lines.contains(&bullet.as_str()),
            "expected {bullet:?} in the text, got: {text:?}"
        );
        let bullet = format!("- {item}");
        assert!(
            markdown_lines.contains(&bullet.as_str()),
            "expected {bullet:?} in the Markdown, got: {markdown:?}"
        );
    }
}

#[test]
fn no_number_loses_its_sign_to_a_bullet() {
    let (text, markdown) = render();
    for value in ["0.2 4.5", "2.9 -4.5", "0.8 3.1", "0.7 1.2", ".5", "12"] {
        assert!(
            !text.contains(&format!("\u{2022} {value}")),
            "{value:?} was read as a bullet item in the text: {text:?}"
        );
        assert!(
            !lines(&markdown)
                .iter()
                .any(|l| l.starts_with(&format!("- {value}"))),
            "{value:?} was read as a bullet item in the Markdown: {markdown:?}"
        );
    }
}

/// The shape-refinement pass rewrites the Markdown; a line that starts with a signed
/// number must survive it unchanged.
#[cfg(feature = "refine")]
#[test]
fn a_signed_number_line_survives_the_refine_pass() {
    let doc = unpdf::parse_bytes(PDF).expect("fixture parses");
    let markdown = to_markdown(&doc, &RenderOptions::default().with_refine()).expect("render");
    let lines = lines(&markdown);
    for signed in SIGNED {
        assert!(
            lines.contains(signed),
            "expected the line {signed:?} after refine, got: {markdown:?}"
        );
    }
}
