//! Text set in a fixed-pitch font is preformatted: its lines are not joined as prose, so a line
//! comment cannot swallow the next statement, and it is written as a fenced code block.

mod common;

use unpdf::model::Block;
use unpdf::render::{to_markdown, to_text, RenderOptions};
use unpdf::PdfParser;

/// A page with Helvetica as `/F1` and `code_font` as `/F2`, whose font dictionary is
/// `code_font` itself (`/BaseFont/... ` and whatever else the case needs).
fn page(content: &[u8], code_font: &str) -> Vec<u8> {
    common::assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(&format!("<</Length {}>>", content.len()), content),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
        format!("<</Type/Font/Subtype/Type1/Encoding/WinAnsiEncoding{code_font}>>").into_bytes(),
    ])
}

const COURIER: &str = "/BaseFont/Courier";

/// A non-standard font whose descriptor says FixedPitch (flag bit 1).
fn flagged(name: &str) -> String {
    format!(
        "/BaseFont/{name}/FontDescriptor<</Type/FontDescriptor/FontName/{name}/Flags 33\
         /FontBBox[0 0 600 800]/ItalicAngle 0/Ascent 800/Descent -200/CapHeight 700/StemV 80>>"
    )
}

fn markdown(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf).unwrap().parse().unwrap();
    to_markdown(&doc, &RenderOptions::default()).unwrap()
}

#[test]
fn a_line_comment_does_not_swallow_the_next_declaration() {
    let content = b"BT /F2 11 Tf 72 700 Td (int first; // first declaration) Tj ET \
        BT /F2 11 Tf 72 686 Td (int second; // second declaration) Tj ET\n";
    for font in [
        COURIER.to_string(),
        "/BaseFont/ABCDEF+Courier-Bold".to_string(),
        flagged("ABCDEF+FiraCode-Regular"),
        flagged("Consolas"),
    ] {
        let md = markdown(&page(content, &font));
        let lines: Vec<_> = md.lines().collect();
        assert!(
            lines.contains(&"int first; // first declaration"),
            "{font}: {md}"
        );
        assert!(
            lines.contains(&"int second; // second declaration"),
            "{font}: {md}"
        );
    }
}

/// A font's name is not what makes it fixed-pitch: a font named like a monospace family that
/// does not declare FixedPitch is read as prose.
#[test]
fn a_monospace_name_alone_is_not_fixed_pitch() {
    let content = b"BT /F2 11 Tf 72 700 Td (This sentence continues) Tj ET \
        BT /F2 11 Tf 72 686 Td (on the following line.) Tj ET\n";
    let md = markdown(&page(content, "/BaseFont/Consolas"));
    assert!(
        md.contains("This sentence continues on the following line."),
        "{md}"
    );
}

/// An OCR layer is drawn invisibly (rendering mode 3) in the engine's stand-in font, which
/// Tesseract declares fixed-pitch. That font says nothing about the page's type: its text is
/// prose.
#[test]
fn an_invisible_text_layer_in_a_fixed_pitch_font_is_prose() {
    let content = b"BT 3 Tr /F2 11 Tf 72 700 Td (This sentence continues) Tj ET         BT 3 Tr /F2 11 Tf 72 686 Td (on the following line.) Tj ET
";
    let md = markdown(&page(content, &flagged("GlyphLessFont")));
    assert!(!md.contains("```"), "{md}");
    assert!(
        md.contains("This sentence continues on the following line."),
        "{md}"
    );
}

#[test]
fn a_comment_only_line_does_not_swallow_the_next_statement() {
    let content = b"BT /F2 11 Tf 72 700 Td (// initialize the counter) Tj ET \
        BT /F2 11 Tf 72 686 Td (int count = 0;) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(md.lines().any(|line| line == "int count = 0;"), "{md}");
}

#[test]
fn ordinary_prose_still_joins_across_physical_lines() {
    let content = b"BT /F1 11 Tf 72 700 Td (This sentence continues) Tj ET \
        BT /F1 11 Tf 72 686 Td (on the following line.) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(
        md.contains("This sentence continues on the following line."),
        "{md}"
    );
}

#[test]
fn inline_monospaced_text_does_not_split_a_prose_paragraph() {
    let content = b"BT /F1 11 Tf 72 700 Td (The name ) Tj /F2 11 Tf (counter) Tj \
        /F1 11 Tf ( in this sentence) Tj ET \
        BT /F1 11 Tf 72 686 Td (continues on the next line.) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(
        md.contains("in this sentence continues on the next line."),
        "{md}"
    );
}

#[test]
fn braces_and_indentation_survive_default_cleanup() {
    let content = b"BT /F2 10 Tf 72 700 Td (void f\\(\\)) Tj ET \
        BT /F2 10 Tf 72 686 Td ({) Tj ET \
        BT /F2 10 Tf 84 672 Td (int count = 0;) Tj ET \
        BT /F2 10 Tf 72 658 Td (}) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(
        md.contains("```\nvoid f()\n{\n  int count = 0;\n}\n```"),
        "{md}"
    );
}

#[test]
fn backticks_in_code_do_not_close_the_generated_fence() {
    let content = b"BT /F2 11 Tf 72 700 Td (```) Tj ET \
        BT /F2 11 Tf 72 686 Td (int count = 0;) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(md.contains("````\n```\nint count = 0;\n````"), "{md}");
}

#[test]
fn blank_lines_keep_a_chained_listing_and_its_indentation() {
    let content = b"BT /F2 10 Tf 72 700 Td (int main\\(\\) {) Tj ET \
        BT /F2 10 Tf 84 686 Td (int a; // test) Tj ET \
        BT /F2 10 Tf 84 672 Td (int b;) Tj ET \
        BT /F2 10 Tf 84 644 Td (int c;) Tj ET \
        BT /F2 10 Tf 84 616 Td (return 0;) Tj ET \
        BT /F2 10 Tf 72 602 Td (}) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(
        md.contains(
            "```\nint main() {\n  int a; // test\n  int b;\n\n  int c;\n\n  return 0;\n}\n```"
        ),
        "{md}"
    );
    assert_eq!(md.matches("```").count(), 2, "{md}");
}

#[test]
fn separated_listings_stay_separate() {
    for middle in [
        "BT /F1 10 Tf 72 672 Td (Explanation between examples.) Tj ET",
        "",
    ] {
        let content = format!(
            "BT /F2 10 Tf 72 700 Td (int a;) Tj ET \
            BT /F2 10 Tf 72 686 Td (int b;) Tj ET {middle} \
            BT /F2 10 Tf 72 602 Td (int c;) Tj ET \
            BT /F2 10 Tf 72 588 Td (int d;) Tj ET"
        );
        let md = markdown(&page(content.as_bytes(), COURIER));
        assert_eq!(md.matches("```").count(), 4, "{md}");
    }
}

/// The code block is the model's; only Markdown fences it. Plain text carries the lines as
/// printed, with no fence characters.
#[test]
fn plain_text_and_the_model_carry_code_without_fences() {
    let content = b"BT /F2 10 Tf 72 700 Td ({) Tj ET \
        BT /F2 10 Tf 84 686 Td (int count = 0;) Tj ET \
        BT /F2 10 Tf 72 672 Td (}) Tj ET\n";
    let doc = PdfParser::from_bytes(&page(content, COURIER))
        .unwrap()
        .parse()
        .unwrap();
    let code: Vec<&str> = doc.pages[0]
        .elements
        .iter()
        .filter_map(|b| match b {
            Block::Code { content } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(code, ["{\n  int count = 0;\n}"]);
    let text = to_text(&doc, &RenderOptions::default()).unwrap();
    assert!(!text.contains("```"), "{text}");
    assert!(text.contains("{\n  int count = 0;\n}"), "{text}");
}

/// A list marker opening a line of code (`- x`) does not make the listing a list.
#[test]
fn a_dash_opening_a_line_of_code_keeps_it_code() {
    let content = b"BT /F2 10 Tf 72 700 Td (- x = 1) Tj ET \
        BT /F2 10 Tf 72 686 Td (- y = 2) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(md.contains("```\n- x = 1\n- y = 2\n```"), "{md}");
}

/// A negative font size (legal: the glyphs are mirrored) neither panics nor stops the lines
/// from reading as code.
#[test]
fn a_negative_font_size_is_read() {
    let content = b"BT /F2 -10 Tf 72 700 Td (int a;) Tj ET \
        BT /F2 -10 Tf 72 686 Td (int b;) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(md.contains("int a;") && md.contains("int b;"), "{md}");
}

/// A descriptor that says FixedPitch with `/Widths` of `widths(code)` for the codes 32..=126.
fn flagged_with_widths(name: &str, widths: impl Fn(u8) -> u32) -> String {
    let list: Vec<String> = (32u8..=126).map(|c| widths(c).to_string()).collect();
    format!(
        "{}/FirstChar 32/LastChar 126/Widths[{}]",
        flagged(name),
        list.join(" ")
    )
}

/// A producer that writes FixedPitch on every face it embeds (Skia: Google Docs, Chrome) does
/// not make the face fixed-pitch: its own widths say it is not. Its prose stays prose.
#[test]
fn a_flag_that_the_fonts_widths_contradict_is_not_believed() {
    let content = b"BT /F2 11 Tf 72 700 Td (This sentence continues) Tj ET \
        BT /F2 11 Tf 72 686 Td (on the following line.) Tj ET\n";
    let font = flagged_with_widths("ABCDEF+Roboto-Regular", |c| 400 + u32::from(c % 7) * 50);
    let md = markdown(&page(content, &font));
    assert!(!md.contains("```"), "{md}");
    assert!(
        md.contains("This sentence continues on the following line."),
        "{md}"
    );
}

/// The same flag with every glyph one advance is a fixed-pitch face: listings still fence.
#[test]
fn a_flag_that_the_fonts_widths_confirm_is_believed() {
    let content = b"BT /F2 11 Tf 72 700 Td (int first; // first declaration) Tj ET \
        BT /F2 11 Tf 72 686 Td (int second; // second declaration) Tj ET\n";
    let font = flagged_with_widths("ABCDEF+FiraCode-Regular", |_| 600);
    let md = markdown(&page(content, &font));
    assert!(
        md.contains("```\nint first; // first declaration\nint second; // second declaration\n```"),
        "{md}"
    );
}

/// A page typed on a typewriter -- every run fixed-pitch, nothing to set a listing apart from --
/// is prose: the typist's line breaks do not stay, and a word hyphenated at the line end is
/// whole again.
#[test]
fn a_typewritten_page_is_prose_not_one_code_block() {
    let content = b"BT /F2 10 Tf 72 700 Td (The sentence wraps at inter-) Tj ET \
        BT /F2 10 Tf 72 688 Td (national boundaries and ends here.) Tj ET \
        BT /F2 10 Tf 72 676 Td (The next sentence follows it on the page.) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(!md.contains("```"), "{md}");
    assert!(
        md.contains("international boundaries and ends here."),
        "{md}"
    );
}

/// A typewritten page that reads as a program is still a listing.
#[test]
fn a_page_of_nothing_but_a_program_is_still_code() {
    let content = b"BT /F2 10 Tf 72 700 Td (// the first declaration) Tj ET \
        BT /F2 10 Tf 72 688 Td (int a = 1;) Tj ET \
        BT /F2 10 Tf 72 676 Td (int b = a + 2;) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(
        md.contains("```\n// the first declaration\nint a = 1;\nint b = a + 2;\n```"),
        "{md}"
    );
}

/// With proportional text on the page the fixed pitch does set a run apart, however plain its
/// words: a command in a manual stays as printed.
#[test]
fn fixed_pitch_among_proportional_text_still_sets_a_run_apart() {
    let content = b"BT /F1 11 Tf 72 740 Td (Run the following command from the project directory and then read the output carefully.) Tj ET \
        BT /F2 11 Tf 72 700 Td (build the project now) Tj ET \
        BT /F2 11 Tf 72 686 Td (and then run the tests) Tj ET\n";
    let md = markdown(&page(content, COURIER));
    assert!(
        md.contains("```\nbuild the project now\nand then run the tests\n```"),
        "{md}"
    );
}
