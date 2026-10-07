//! A chapter number set on its own line above the title is part of the title.
//!
//! Book classes open a chapter with its number in a large face and the title on the line
//! below. Read line by line, the number is a lone digit — too short to be a heading — and the
//! title loses it: `**4**` then `## Basis Fields` instead of one `# 4 Basis Fields`.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

fn page(number_size: u32, number_y: u32) -> Vec<u8> {
    let content = format!(
        "BT /F2 {number_size} Tf 72 {number_y} Td (4) Tj ET \
         BT /F2 17 Tf 72 680 Td (Basis Fields) Tj ET \
         BT /F1 10 Tf 72 650 Td (A vector field may be written as a linear combination of basis fields.) Tj ET \
         BT /F1 10 Tf 72 638 Td (If n is the dimension, then any set of n independent fields is a basis.) Tj ET \
         BT /F1 10 Tf 72 626 Td (The coordinate basis is an example of a basis, and there are others too.) Tj ET\n"
    );
    common::assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ),
        b"<</Type/Font/Subtype/Type1/BaseFont/Times-Roman/Encoding/WinAnsiEncoding>>".to_vec(),
        b"<</Type/Font/Subtype/Type1/BaseFont/Times-Bold/Encoding/WinAnsiEncoding>>".to_vec(),
    ])
}

fn markdown(pdf: &[u8]) -> String {
    let doc = PdfParser::from_bytes(pdf).and_then(|p| p.parse()).unwrap();
    to_markdown(&doc, &RenderOptions::default()).unwrap()
}

#[test]
fn a_chapter_number_above_its_title_joins_it() {
    let md = markdown(&page(25, 705));
    assert!(md.contains("# 4 Basis Fields"), "{md}");
    assert!(!md.contains("**4**"), "{md}");
}

#[test]
fn a_small_number_above_a_title_stays_apart() {
    let md = markdown(&page(9, 705));
    assert!(!md.contains("4 Basis Fields"), "{md}");
    assert!(md.contains("Basis Fields"), "{md}");
}
