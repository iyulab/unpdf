//! Rules that frame text are not a table: the text inside keeps its lines.
//!
//! A page of typewritten text inside a ruled box (a printout, a form's instructions) gives a
//! grid whose cells are all empty but one. Read as a table that cell was one run of text; read
//! as the page's text, the lines stay as printed.

mod common;

use common::{assemble, stream_object};

fn courier_page(content: &str) -> Vec<u8> {
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 400]/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content.as_bytes()),
        b"<</Type/Font/Subtype/Type1/BaseFont/Courier>>".to_vec(),
    ])
}

/// A box ruled into nine cells, every line of text in the middle one -- a report's lines drawn
/// with `|` and `=`, which read as a listing, not as a typewritten page of prose.
fn framed_lines() -> String {
    let rules = "0.5 w 50 50 300 300 re S 150 50 m 150 350 l S 250 50 m 250 350 l S                  50 150 m 350 150 l S 50 250 m 350 250 l S";
    let text = "BT /F1 6 Tf 155 240 Td 8 TL (| first line = report |) Tj T* (| second = aligned    here |) Tj T* (| third line = end |) Tj ET";
    format!("{rules} {text}")
}

#[test]
fn text_in_a_ruled_frame_keeps_its_lines() {
    let pdf = courier_page(&framed_lines());
    let doc = unpdf::parse_bytes(&pdf).expect("parses");
    let md = unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default()).expect("renders");
    assert!(
        md.contains(
            "| first line = report |
| second = aligned    here |
| third line = end |"
        ),
        "the three lines must stay lines, got {md:?}"
    );
}
