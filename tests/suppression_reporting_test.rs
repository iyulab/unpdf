//! A text run the decoder discards must be reported, not silently dropped.
//!
//! When a font's character codes cannot be resolved, emitting the raw bytes would
//! produce mojibake rather than text — so the run is discarded. That policy is
//! deliberate and stays. What is not acceptable is the extraction then reporting
//! success with no indication that content went missing: a consumer indexing the
//! result cannot tell "the document did not say this" from "we could not read it".
//!
//! PDFs are assembled byte-by-byte here rather than read from `test-files/`
//! (gitignored, so fixture-based tests silently skip in CI).

use unpdf::PdfParser;

/// Assemble a structurally valid PDF from 1-indexed object bodies, computing a
/// traditional xref table over their real offsets.
fn assemble(objects: &[String]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());

    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }

    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \r\n");
    for offset in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \r\n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\n", objects.len() + 1).as_bytes(),
    );
    out.extend_from_slice(format!("startxref\n{}\n%%EOF", xref_offset).as_bytes());
    out
}

/// A one-page document drawing `literal` with the font object given as `font_body`.
fn one_page_pdf(font_body: &str, literal: &str, extra_objects: &[String]) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 720 Td ({}) Tj ET", literal);
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ),
        font_body.to_string(),
    ];
    objects.extend_from_slice(extra_objects);
    assemble(&objects)
}

/// An Identity-H composite font with no `ToUnicode` map and no embedded cmap —
/// the decoder has no way to turn its CIDs into characters.
fn unresolvable_composite_pdf() -> Vec<u8> {
    one_page_pdf(
        "<< /Type /Font /Subtype /Type0 /BaseFont /NoMap /Encoding /Identity-H \
         /DescendantFonts [6 0 R] >>",
        // Two CIDs. Byte-wise Latin-1 reading of these is categorically wrong.
        "\\001\\102\\001\\103",
        &["<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NoMap \
           /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>"
            .to_string()],
    )
}

/// An ordinary Type1 font whose text decodes cleanly — nothing to suppress.
fn readable_pdf() -> Vec<u8> {
    one_page_pdf(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "Readable text",
        &[],
    )
}

#[test]
fn unresolvable_composite_font_reports_suppressed_runs() {
    let doc = PdfParser::from_bytes(&unresolvable_composite_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    assert!(
        doc.extraction_quality.suppressed_text_runs > 0,
        "a discarded run must be counted, got quality {:?}",
        doc.extraction_quality
    );
}

#[test]
fn suppressed_runs_produce_a_warning() {
    let doc = PdfParser::from_bytes(&unresolvable_composite_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    let warning = doc
        .extraction_quality
        .warning_message()
        .expect("a document that lost text must warn");
    assert!(
        warning.contains("unreadable text run"),
        "warning must name what was lost, got {warning:?}"
    );
}

#[test]
fn readable_document_reports_no_suppression() {
    let doc = PdfParser::from_bytes(&readable_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    assert_eq!(
        doc.extraction_quality.suppressed_text_runs, 0,
        "an ordinary document must not report suppression — false positives here \
         would make the signal useless"
    );
}

/// The per-page count is what the document-level total is built from, so a page that
/// lost runs has to say so on its own — the FFI page diagnostics expose this field.
#[test]
fn suppression_is_visible_on_the_page_that_lost_the_runs() {
    let doc = PdfParser::from_bytes(&unresolvable_composite_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    let total: usize = doc.pages.iter().map(|p| p.suppressed_text_runs).sum();
    assert_eq!(
        total, doc.extraction_quality.suppressed_text_runs,
        "the document total must be the sum of its pages"
    );
    assert!(
        total > 0,
        "the page must carry the count, not just the total"
    );
}

/// Forward compatibility: the field is additive JSON, so a consumer that does not
/// know it still parses the payload. Guards the ABI claim made in the docs.
#[test]
fn quality_serialises_the_new_field() {
    let doc = PdfParser::from_bytes(&unresolvable_composite_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    let json = serde_json::to_string(&doc.extraction_quality).expect("quality must serialise");
    assert!(
        json.contains("suppressed_text_runs"),
        "the counter must reach the JSON surface, got {json}"
    );
}

/// A page that loses runs in three different fonts: a simple font that decodes to
/// binary noise (first), an unresolvable composite font that loses two runs, and an
/// unresolvable composite font with no `/BaseFont` at all.
fn three_unreadable_fonts_pdf() -> Vec<u8> {
    let content = "BT /F2 12 Tf 72 720 Td (\\001\\002\\003\\004\\005\\006) Tj \
                   /F1 12 Tf (\\001\\102) Tj (\\001\\103) Tj \
                   /F3 12 Tf (\\001\\104) Tj ET";
    let cid_system = "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >>";
    assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R /F2 7 0 R /F3 8 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ),
        "<< /Type /Font /Subtype /Type0 /BaseFont /NoMap /Encoding /Identity-H \
         /DescendantFonts [6 0 R] >>"
            .to_string(),
        format!("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NoMap {cid_system} >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [9 0 R] >>"
            .to_string(),
        format!("<< /Type /Font /Subtype /CIDFontType2 {cid_system} >>"),
    ])
}

/// The consumer-facing question: which font, and why. One entry per font and reason, in
/// order of first loss, the resource name standing in for a missing `/BaseFont`.
#[test]
fn unreadable_fonts_name_the_font_and_the_reason() {
    use unpdf::parser::backend::TextSuppression;

    let doc = PdfParser::from_bytes(&three_unreadable_fonts_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");
    let page = &doc.pages[0];

    let seen: Vec<(&str, TextSuppression, usize)> = page
        .unreadable_fonts
        .iter()
        .map(|f| (f.name.as_str(), f.reason, f.runs))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("Helvetica", TextSuppression::BinaryDensity, 1),
            ("NoMap", TextSuppression::CompositeUnresolved, 2),
            ("F3", TextSuppression::CompositeUnresolved, 1),
        ]
    );
}

/// The attribution must account for every discarded run, per page and for the document.
#[test]
fn unreadable_font_runs_sum_to_the_suppressed_run_count() {
    for pdf in [unresolvable_composite_pdf(), three_unreadable_fonts_pdf()] {
        let doc = PdfParser::from_bytes(&pdf)
            .and_then(|p| p.parse())
            .expect("fixture must parse");
        let attributed: usize = doc
            .pages
            .iter()
            .flat_map(|p| &p.unreadable_fonts)
            .map(|f| f.runs)
            .sum();
        assert!(attributed > 0);
        assert_eq!(attributed, doc.extraction_quality.suppressed_text_runs);
        for page in &doc.pages {
            let page_sum: usize = page.unreadable_fonts.iter().map(|f| f.runs).sum();
            assert_eq!(page_sum, page.suppressed_text_runs);
        }
    }
}

#[test]
fn readable_page_lists_no_unreadable_fonts() {
    let doc = PdfParser::from_bytes(&readable_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");
    assert!(doc.pages[0].unreadable_fonts.is_empty());
}

/// The reason is a public string contract: the same text on every surface.
#[test]
fn unreadable_font_serialises_with_a_stable_reason_string() {
    let doc = PdfParser::from_bytes(&three_unreadable_fonts_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");
    let json = serde_json::to_value(&doc.pages[0]).expect("page must serialise");
    assert_eq!(
        json["unreadable_fonts"][0],
        serde_json::json!({"name": "Helvetica", "reason": "binary_density", "runs": 1})
    );
    assert_eq!(
        json["unreadable_fonts"][1]["reason"],
        "composite_unresolved"
    );

    // Omitted when empty, like the page's other optional diagnostics.
    let clean = PdfParser::from_bytes(&readable_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");
    let json = serde_json::to_value(&clean.pages[0]).expect("page must serialise");
    assert!(json.get("unreadable_fonts").is_none());
}
