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

/// A page that draws, in two fonts carrying their own glyphs, codes that have no character:
/// a Type 3 font whose `ToUnicode` maps code 1 to a whole emoji cluster and leaves out code
/// 2, the cluster's other glyph, and a symbolic TrueType font that embeds its program and
/// gives no `/Encoding` or `ToUnicode` at all.
fn unmapped_codes_pdf() -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 720 Td [<01> -64 <02>] TJ /F2 12 Tf (\\001) Tj ET";
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                1 begincodespacerange <00> <FF> endcodespacerange \
                1 beginbfchar <01> <D83DDC68200DD83DDC69> endbfchar endcmap end end";
    let glyph = "0 0 d0";
    let program: String = include_bytes!("fixtures/square.ttf")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R /F2 9 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /g1 6 0 R /g2 7 0 R >> \
         /Encoding << /Type /Encoding /Differences [1 /g1 /g2] >> \
         /FirstChar 1 /LastChar 2 /Widths [500 0] /ToUnicode 8 0 R >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{glyph}\nendstream", glyph.len()),
        format!("<< /Length {} >>\nstream\n{glyph}\nendstream", glyph.len()),
        format!("<< /Length {} >>\nstream\n{cmap}\nendstream", cmap.len()),
        "<< /Type /Font /Subtype /TrueType /BaseFont /UnpdfSquare /FirstChar 1 /LastChar 1 \
         /Widths [1000] /FontDescriptor 10 0 R >>"
            .to_string(),
        "<< /Type /FontDescriptor /FontName /UnpdfSquare /Flags 4 /FontBBox [0 0 1000 800] \
         /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 800 /StemV 80 /FontFile2 11 0 R >>"
            .to_string(),
        format!(
            "<< /Filter /ASCIIHexDecode /Length {} >>\nstream\n{program}>\nendstream",
            program.len() + 1
        ),
    ])
}

/// Codes a font draws but gives no character are no misread: they have a reason of their
/// own, still counted, since the page shows glyphs its text does not carry.
#[test]
fn codes_a_font_gives_no_character_are_unmapped_not_binary_noise() {
    use unpdf::parser::backend::TextSuppression;

    let doc = PdfParser::from_bytes(&unmapped_codes_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");
    assert_eq!(doc.plain_text().trim(), "\u{1f468}\u{200d}\u{1f469}");

    let page = &doc.pages[0];
    let seen: Vec<(&str, TextSuppression, usize)> = page
        .unreadable_fonts
        .iter()
        .map(|f| (f.name.as_str(), f.reason, f.runs))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("F1", TextSuppression::UnmappedCodes, 1),
            ("UnpdfSquare", TextSuppression::UnmappedCodes, 1),
        ]
    );
    let json = serde_json::to_value(page).expect("page must serialise");
    assert_eq!(json["unreadable_fonts"][0]["reason"], "unmapped_codes");
}

fn composite_pdf(
    shown: &str,
    encoding: &str,
    ordering: &str,
    to_unicode: &str,
    extra: &[String],
) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 720 Td {shown} ET");
    let to_unicode = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap {to_unicode} endcmap end end"
    );
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /Partial /Encoding {encoding} \
             /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>"
        ),
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Partial \
             /CIDSystemInfo << /Registry (Adobe) /Ordering ({ordering}) /Supplement 0 >> \
             /DW 500 /W [6 [0]] /W2 [7 [0 250 880]] >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{to_unicode}\nendstream",
            to_unicode.len()
        ),
    ];
    objects.extend_from_slice(extra);
    assemble(&objects)
}

fn partially_mapped_composite_pdf(shown: &str) -> Vec<u8> {
    composite_pdf(
        shown,
        "/Identity-H",
        "Identity",
        "1 begincodespacerange <0000> <FFFF> endcodespacerange \
         5 beginbfchar <0001> <0050> <0002> <006C> <0003> <0061> <0004> <006E> \
         <0005> <D83DDC68200DD83DDC69> endbfchar",
        &[],
    )
}

fn parsed_page(pdf: &[u8]) -> (String, unpdf::model::Page) {
    let doc = PdfParser::from_bytes(pdf)
        .and_then(|p| p.parse())
        .expect("fixture must parse");
    (doc.plain_text().trim().to_string(), doc.pages[0].clone())
}

#[test]
fn an_advancing_code_a_composite_font_leaves_out_is_counted_as_undecoded() {
    let (text, page) = parsed_page(&partially_mapped_composite_pdf("<00010002000300040200> Tj"));
    assert_eq!(text, "Plan");
    assert_eq!(page.undecoded_text_codes, 1);
    assert_eq!(page.suppressed_text_runs, 0);
    assert!(page.unreadable_fonts.is_empty());
    let json = serde_json::to_value(&page).expect("page must serialise");
    assert_eq!(json["undecoded_text_codes"], 1);
}

#[test]
fn a_zero_width_code_a_composite_font_leaves_out_is_not_undecoded() {
    let (text, page) = parsed_page(&partially_mapped_composite_pdf(
        "<000100020003000400050006> Tj",
    ));
    assert_eq!(text, "Plan\u{1f468}\u{200d}\u{1f469}");
    assert_eq!(page.undecoded_text_codes, 0);
    assert_eq!(page.suppressed_text_runs, 0);
    let json = serde_json::to_value(&page).expect("page must serialise");
    assert!(json.get("undecoded_text_codes").is_none());
}

#[test]
fn a_run_the_clip_hides_counts_no_undecoded_code() {
    let (text, page) = parsed_page(&partially_mapped_composite_pdf(
        "<00010002> Tj ET q 0 0 10 10 re W n \
         BT /F1 12 Tf 300 400 Td <00030200> Tj ET Q BT",
    ));
    assert_eq!(text, "Pl");
    assert_eq!(page.clipped_text_runs, 1);
    assert_eq!(page.undecoded_text_codes, 0);
}

#[test]
fn a_kerned_string_of_codes_left_out_is_judged_by_each_element() {
    use unpdf::parser::backend::TextSuppression;

    let (text, page) = parsed_page(&partially_mapped_composite_pdf(
        "[<0001000200030004> -20 <0200>] TJ",
    ));
    assert_eq!(text, "Plan");
    assert_eq!(page.undecoded_text_codes, 0);
    assert_eq!(page.suppressed_text_runs, 1);
    assert_eq!(
        page.unreadable_fonts[0].reason,
        TextSuppression::CompositeUnresolved
    );

    let (text, page) = parsed_page(&partially_mapped_composite_pdf(
        "[<0005> -64 <0006> 777 <0007>] TJ",
    ));
    assert_eq!(text, "\u{1f468}\u{200d}\u{1f469}");
    assert_eq!(page.undecoded_text_codes, 0);
    assert_eq!(
        page.suppressed_text_runs, 1,
        "code 7 advances, code 6 does not"
    );
}

#[test]
fn identity_h_codes_are_two_bytes_whatever_the_to_unicode_declares() {
    let pdf = |shown: &str| {
        composite_pdf(
            shown,
            "/Identity-H",
            "Identity",
            "1 begincodespacerange <00> <FF> endcodespacerange \
             4 beginbfchar <01> <0050> <02> <006C> <03> <0061> <04> <006E> endbfchar",
            &[],
        )
    };
    let (text, page) = parsed_page(&pdf("<0001000200030004> Tj"));
    assert_eq!(text, "Plan");
    assert_eq!(page.undecoded_text_codes, 0);

    let (text, page) = parsed_page(&pdf("<00010002000300040200> Tj"));
    assert_eq!(text, "Plan");
    assert_eq!(page.undecoded_text_codes, 1);
}

#[test]
fn a_code_left_out_of_a_vertical_font_advances_down_the_page() {
    let pdf = |shown: &str| {
        composite_pdf(
            shown,
            "/Identity-V",
            "Identity",
            "1 begincodespacerange <0000> <FFFF> endcodespacerange \
             1 beginbfchar <0001> <0050> endbfchar",
            &[],
        )
    };
    let (text, page) = parsed_page(&pdf("<00010006> Tj"));
    assert_eq!(text, "P");
    assert_eq!(page.undecoded_text_codes, 1);

    let (text, page) = parsed_page(&pdf("<00010007> Tj"));
    assert_eq!(text, "P");
    assert_eq!(page.undecoded_text_codes, 0);
}

fn one_byte_encoding_pdf(shown: &str, to_unicode: &str, dict: &str) -> Vec<u8> {
    let encoding = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                    /CMapName /OneByte def 1 begincodespacerange <00> <FF> endcodespacerange \
                    1 begincidrange <00> <FF> 0 endcidrange endcmap end end";
    composite_pdf(
        shown,
        "8 0 R",
        "Identity",
        to_unicode,
        &[format!(
            "<< /Type /CMap /CMapName /OneByte {dict} /Length {} >>\nstream\n{encoding}\nendstream",
            encoding.len()
        )],
    )
}

#[test]
fn a_one_byte_code_left_out_under_an_embedded_encoding_is_never_its_ascii() {
    let keyed = |keys: [u8; 4]| {
        format!(
            "4 beginbfchar <{:02X}> <0050> <{:02X}> <006C> <{:02X}> <0061> <{:02X}> <006E> endbfchar",
            keys[0], keys[1], keys[2], keys[3]
        )
    };
    for (shown, keys, undecoded, suppressed) in [
        ("<4142434445> Tj", [0x41, 0x42, 0x43, 0x44], 1, 0),
        ("[<41424344> -20 <45>] TJ", [0x41, 0x42, 0x43, 0x44], 0, 1),
        ("<0102030441> Tj", [0x01, 0x02, 0x03, 0x04], 1, 0),
    ] {
        for dict in [
            "",
            "/UseCMap /GBK-EUC-H",
            "/UseCMap /UniJIS-UCS2-H",
            "/UseCMap /NoSuchCMap-H",
        ] {
            for codespace in ["", "1 begincodespacerange <0000> <FFFF> endcodespacerange "] {
                let to_unicode = format!("{codespace}{}", keyed(keys));
                let (text, page) = parsed_page(&one_byte_encoding_pdf(shown, &to_unicode, dict));
                assert_eq!(text, "Plan", "shown: {shown}, {dict}, {to_unicode}");
                assert_eq!(
                    page.undecoded_text_codes, undecoded,
                    "shown: {shown}, {dict}, {to_unicode}"
                );
                assert_eq!(page.suppressed_text_runs, suppressed);
                assert_eq!(page.unreadable_fonts.len(), suppressed);
            }
        }
    }
}

#[test]
fn a_code_the_to_unicode_leaves_out_reads_through_the_fonts_own_data() {
    let pdf = |shown: &str| {
        composite_pdf(
            shown,
            "/Identity-H",
            "Korea1",
            "1 begincodespacerange <0000> <FFFF> endcodespacerange \
             1 beginbfchar <0001> <0050> endbfchar",
            &[],
        )
    };
    let (text, page) = parsed_page(&pdf("<00010022> Tj"));
    assert_eq!(text, "PA");
    assert_eq!(page.undecoded_text_codes, 0);

    let (text, page) = parsed_page(&pdf("<0022> Tj"));
    assert_eq!(text, "A");
    assert_eq!(page.undecoded_text_codes, 0);

    let pdf = |shown: &str| {
        composite_pdf(
            shown,
            "/KSC-EUC-H",
            "Korea1",
            "1 begincodespacerange <0000> <FFFF> endcodespacerange \
             1 beginbfchar <B1DB> <D55C> endbfchar",
            &[],
        )
    };
    let (text, page) = parsed_page(&pdf("<B1DB41> Tj"));
    assert_eq!(text, "\u{d55c}A");
    assert_eq!(page.undecoded_text_codes, 0);

    let (text, page) = parsed_page(&pdf("<B0A1> Tj"));
    assert_eq!(text, "\u{ac00}");
    assert_eq!(page.undecoded_text_codes, 0);
}
