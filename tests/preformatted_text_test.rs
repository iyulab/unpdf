//! Physical line boundaries in monospaced text must not turn code into comments.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

fn page(content: &[u8], code_font: &str) -> Vec<u8> {
    common::assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(&format!("<</Length {}>>", content.len()), content),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
        format!("<</Type/Font/Subtype/Type1/BaseFont/{code_font}/Encoding/WinAnsiEncoding>>")
            .into_bytes(),
    ])
}

fn markdown(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf).unwrap().parse().unwrap();
    to_markdown(&doc, &RenderOptions::default()).unwrap()
}

#[test]
fn a_line_comment_does_not_swallow_the_next_declaration() {
    let content = b"BT /F2 11 Tf 72 700 Td (int first; // first declaration) Tj ET \
        BT /F2 11 Tf 72 686 Td (int second; // second declaration) Tj ET\n";
    for font in ["Courier", "ABCDEF+FiraCode-Regular", "Consolas"] {
        let md = markdown(&page(content, font));
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

#[test]
fn a_comment_only_line_does_not_swallow_the_next_statement() {
    let content = b"BT /F2 11 Tf 72 700 Td (// initialize the counter) Tj ET \
        BT /F2 11 Tf 72 686 Td (int count = 0;) Tj ET\n";
    let md = markdown(&page(content, "Courier"));
    assert!(md.lines().any(|line| line == "int count = 0;"), "{md}");
}

#[test]
fn ordinary_prose_still_joins_across_physical_lines() {
    let content = b"BT /F1 11 Tf 72 700 Td (This sentence continues) Tj ET \
        BT /F1 11 Tf 72 686 Td (on the following line.) Tj ET\n";
    let md = markdown(&page(content, "Courier"));
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
    let md = markdown(&page(content, "Courier"));
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
    let md = markdown(&page(content, "Courier"));
    assert!(
        md.contains("```\nvoid f()\n{\n  int count = 0;\n}\n```"),
        "{md}"
    );
}

#[test]
fn backticks_in_code_do_not_close_the_generated_fence() {
    let content = b"BT /F2 11 Tf 72 700 Td (```) Tj ET \
        BT /F2 11 Tf 72 686 Td (int count = 0;) Tj ET\n";
    let md = markdown(&page(content, "Courier"));
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
    let md = markdown(&page(content, "Courier"));
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
        let md = markdown(&page(content.as_bytes(), "Courier"));
        assert_eq!(md.matches("```").count(), 4, "{md}");
    }
}
