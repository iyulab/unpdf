//! What "structure only" leaves out, and what it keeps.
//!
//! `ParseOptions::with_text(false)` is an output contract, and until now nothing pinned it:
//! no test anywhere exercised the mode, which is how a sibling variant (`TextOnly`) stayed
//! dead for entire releases without anyone noticing. The rule this file fixes is the one
//! the parser already followed — **the page survives, its content blocks are not built** —
//! stated as assertions so it cannot drift, and so the sibling parsers have something
//! concrete to match.
//!
//! The PDF is built here rather than read from a fixture directory, so the test runs
//! everywhere instead of skipping when a fixture is absent.

use unpdf::{parse_bytes, parse_bytes_with_options, ParseOptions};

/// A one-page PDF whose content stream draws a short line of text.
fn minimal_pdf() -> Vec<u8> {
    let stream = b"BT /F1 12 Tf 20 100 Td (Hello unpdf) Tj ET";
    let bodies: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R\
           /Resources<</Font<</F1 5 0 R>>>>>>"
            .to_vec(),
        {
            let mut v = format!("<</Length {}>>stream\n", stream.len()).into_bytes();
            v.extend_from_slice(stream);
            v.extend_from_slice(b"\nendstream");
            v
        },
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ];

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"endobj\n");
    }

    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", bodies.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer<</Size {}/Root 1 0 R>>\nstartxref\n{}\n%%EOF",
            bodies.len() + 1,
            xref
        )
        .as_bytes(),
    );
    out
}

/// The baseline the structure-only assertions are measured against. Without it a mode that
/// produced nothing at all would satisfy every "is empty" assertion below.
#[test]
fn a_full_parse_produces_content_blocks() {
    let doc = parse_bytes(&minimal_pdf()).expect("the built PDF must parse");

    assert_eq!(doc.pages.len(), 1);
    assert!(
        !doc.pages[0].elements.is_empty(),
        "the baseline document must carry content, or the structure-only test is vacuous"
    );
}

/// The contract: the page is still there, with its identity intact. Only its content is not.
#[test]
fn structure_only_keeps_the_page_and_drops_its_content() {
    let data = minimal_pdf();
    let full = parse_bytes(&data).expect("the built PDF must parse");
    let structure = parse_bytes_with_options(&data, ParseOptions::new().with_text(false))
        .expect("the built PDF must parse without text extraction");

    assert_eq!(
        structure.pages.len(),
        full.pages.len(),
        "structure-only must not change how many pages a document has"
    );

    let (s, f) = (&structure.pages[0], &full.pages[0]);
    assert_eq!(s.number, f.number, "the page keeps its number");
    assert_eq!(s.width, f.width, "the page keeps its dimensions");
    assert_eq!(s.height, f.height);
    assert!(
        s.elements.is_empty(),
        "structure-only must not build content blocks, got {} of them",
        s.elements.len()
    );
}

/// Text extraction and resource extraction are separate axes. They were not always: asking
/// for structure used to suppress resources as a side effect, so "structure, but keep the
/// images" could not be expressed at all.
#[test]
fn structure_only_does_not_decide_resource_extraction() {
    let data = minimal_pdf();
    let opts = ParseOptions::new().with_text(false).with_resources(true);
    let doc = parse_bytes_with_options(&data, opts).expect("the built PDF must parse");

    // This document embeds no images, so the inventory is legitimately empty -- what is
    // asserted here is that asking for both is accepted and parses, not that a resource
    // appears out of nothing.
    assert_eq!(doc.pages.len(), 1);
    assert!(doc.pages[0].elements.is_empty());
}
