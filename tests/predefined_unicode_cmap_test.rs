//! Composite fonts that name a predefined Unicode CMap and embed nothing.
//!
//! A Type 0 font may leave the glyphs to the reader: no `/ToUnicode`, no font program,
//! and an `/Encoding` that names one of Adobe's predefined CMaps (ISO 32000-1 §9.7.5.2).
//! Under a Unicode CMap (`UniKS-UCS2-H`, `UniJIS-UTF16-V`, ...) the character codes are
//! the text itself, so nothing else is needed to read it.
//!
//! `/DescendantFonts` holds the CIDFont dictionary either by reference or inline — both
//! are valid, and the fixtures (written by ReportLab, see
//! `tests/fixtures/make_cid_font_fixtures.py`) use the inline form.

use std::ops::ControlFlow;

use unpdf::parser::backend::RawBackend;
use unpdf::parser::{LayoutAnalyzer, TextSpan};
use unpdf::{PageStreamOptions, ParseEvent, PdfParser};

const KOREA1_UCS2_H: &[u8] = include_bytes!("fixtures/cid-korea1-ucs2-h.pdf");
const JAPAN1_UCS2_H: &[u8] = include_bytes!("fixtures/cid-japan1-ucs2-h.pdf");
const JAPAN1_UCS2_V: &[u8] = include_bytes!("fixtures/cid-japan1-ucs2-v.pdf");
const GB1_UCS2_H: &[u8] = include_bytes!("fixtures/cid-gb1-ucs2-h.pdf");
const CNS1_UCS2_H: &[u8] = include_bytes!("fixtures/cid-cns1-ucs2-h.pdf");

fn parse(pdf: &[u8]) -> unpdf::Document {
    unpdf::parse_bytes(pdf).expect("fixture parses")
}

/// Every line must come back whole, and nothing may have been discarded.
fn assert_reads(pdf: &[u8], lines: &[&str]) {
    let doc = parse(pdf);
    let text = doc.plain_text();
    for line in lines {
        assert!(text.contains(line), "expected {line:?} in {text:?}");
    }
    assert_eq!(
        doc.extraction_quality.suppressed_text_runs, 0,
        "no run may be discarded, got {text:?}"
    );
}

#[test]
fn korean_text_in_a_non_embedded_font_under_uniks_ucs2_h() {
    assert_reads(
        KOREA1_UCS2_H,
        &["테스트 문서 - PDF", "프로젝트 코드명", "예산: 2,300,000원"],
    );
}

#[test]
fn japanese_text_in_a_non_embedded_font_under_unijis_ucs2_h() {
    assert_reads(JAPAN1_UCS2_H, &["日本語のテスト文書", "こんにちは世界"]);
}

#[test]
fn vertical_writing_mode_reads_the_same_codes() {
    assert_reads(JAPAN1_UCS2_V, &["縦書きのテスト"]);
}

#[test]
fn simplified_chinese_text_under_unigb_ucs2_h() {
    assert_reads(GB1_UCS2_H, &["中文测试文档", "你好世界"]);
}

/// ReportLab names `UniGB-UCS2-H` for its Adobe-CNS1 font: the CMap's collection and
/// the CIDFont's disagree. The codes are Unicode either way.
#[test]
fn traditional_chinese_text_whose_cmap_names_another_collection() {
    assert_reads(CNS1_UCS2_H, &["中文測試文件", "你好世界"]);
}

/// The page-at-a-time path decodes through the same fonts as the whole-document one.
#[test]
fn the_page_stream_reads_the_same_text() {
    let parser = PdfParser::from_bytes(KOREA1_UCS2_H).expect("fixture opens");
    let mut pages = Vec::new();
    let quality = parser
        .for_each_page(PageStreamOptions::default(), |event| {
            if let ParseEvent::PageParsed(page) = event {
                pages.push(page.plain_text());
            }
            ControlFlow::Continue(())
        })
        .expect("stream runs");
    assert_eq!(pages.len(), 1);
    assert!(pages[0].contains("테스트 문서 - PDF"), "got {:?}", pages[0]);
    assert_eq!(quality.suppressed_text_runs, 0);
}

// ---------------------------------------------------------------------------
// Synthetic documents: one font, one encoding, chosen bytes.
// ---------------------------------------------------------------------------

/// A one-page PDF whose only font is a non-embedded Type 0 font under `encoding`, with
/// its CIDFont written inline (`inline`) or as its own object, and `content` as the
/// page's content stream.
fn type0_pdf(encoding: &str, ordering: &str, inline: bool, w: &str, content: &[u8]) -> Vec<u8> {
    let cid_font = format!(
        "<</Type/Font/Subtype/CIDFontType0/BaseFont/StandIn\
         /CIDSystemInfo<</Registry(Adobe)/Ordering({ordering})/Supplement 0>>\
         /FontDescriptor<</Type/FontDescriptor/FontName/StandIn/Flags 4\
         /FontBBox[0 -120 1000 880]/ItalicAngle 0/Ascent 880/Descent -120\
         /CapHeight 700/StemV 80>>/DW 1000/W[{w}]>>"
    );
    let descendants = if inline {
        format!("[{cid_font}]")
    } else {
        "[6 0 R]".to_string()
    };
    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 100]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        [
            format!("<</Length {}>>\nstream\n", content.len()).as_bytes(),
            content,
            b"\nendstream",
        ]
        .concat(),
        format!(
            "<</Type/Font/Subtype/Type0/BaseFont/StandIn\
             /DescendantFonts{descendants}/Encoding/{encoding}>>"
        )
        .into_bytes(),
    ];
    if !inline {
        objects.push(cid_font.into_bytes());
    }

    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_start = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF\n")
            .as_bytes(),
    );
    pdf
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

fn utf32be(text: &str) -> Vec<u8> {
    text.chars()
        .flat_map(|c| u32::from(c).to_be_bytes())
        .collect()
}

fn one_run(code_bytes: &[u8]) -> Vec<u8> {
    format!("BT /F1 12 Tf 20 50 Td <{}> Tj ET", hex(code_bytes)).into_bytes()
}

fn text_of(pdf: &[u8]) -> (String, usize) {
    let doc = parse(pdf);
    (
        doc.plain_text(),
        doc.extraction_quality.suppressed_text_runs,
    )
}

#[test]
fn an_inline_and_a_referenced_cid_font_read_alike() {
    for inline in [true, false] {
        let pdf = type0_pdf(
            "UniKS-UCS2-H",
            "Korea1",
            inline,
            "",
            &one_run(&utf16be("한글 문서")),
        );
        let (text, suppressed) = text_of(&pdf);
        assert!(text.contains("한글 문서"), "inline={inline}: got {text:?}");
        assert_eq!(suppressed, 0, "inline={inline}");
    }
}

/// Every Unicode encoding Adobe's predefined CMaps use, in both writing modes. The
/// sample carries a character outside the Basic Multilingual Plane where the encoding
/// can express one (UCS-2 cannot).
#[test]
fn every_unicode_cmap_family_decodes_in_both_writing_modes() {
    let bmp = "日本語";
    let astral = "𠮷野家";
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("UniJIS-UCS2", utf16be(bmp), bmp),
        ("UniJIS-UCS2-HW", utf16be(bmp), bmp),
        ("UniJIS-UTF16", utf16be(astral), astral),
        ("UniJIS2004-UTF16", utf16be(astral), astral),
        ("UniJIS-UTF8", astral.as_bytes().to_vec(), astral),
        ("UniJIS-UTF32", utf32be(astral), astral),
        ("UniJISX0213-UTF32", utf32be(astral), astral),
    ];
    for (base, bytes, expected) in cases {
        for mode in ["H", "V"] {
            let encoding = format!("{base}-{mode}");
            let (text, suppressed) =
                text_of(&type0_pdf(&encoding, "Japan1", true, "", &one_run(&bytes)));
            assert!(text.contains(expected), "{encoding}: got {text:?}");
            assert_eq!(suppressed, 0, "{encoding}");
        }
    }
    for (encoding, ordering, sample) in [
        ("UniKS-UTF16-H", "Korea1", "한국어"),
        ("UniGB-UTF16-H", "GB1", "简体字"),
        ("UniCNS-UTF16-H", "CNS1", "繁體字"),
        ("UniCNS-UCS2-V", "CNS1", "繁體字"),
    ] {
        let (text, _) = text_of(&type0_pdf(
            encoding,
            ordering,
            true,
            "",
            &one_run(&utf16be(sample)),
        ));
        assert!(text.contains(sample), "{encoding}: got {text:?}");
    }
}

/// A malformed code — here a lone high surrogate — costs that code, not the run.
#[test]
fn a_malformed_code_does_not_discard_its_neighbours() {
    let mut bytes = utf16be("한글");
    bytes.extend_from_slice(&[0xD8, 0x00]);
    bytes.extend_from_slice(&utf16be("문서"));
    let (text, _) = text_of(&type0_pdf(
        "UniKS-UCS2-H",
        "Korea1",
        true,
        "",
        &one_run(&bytes),
    ));
    assert!(
        text.contains("한글") && text.contains("문서"),
        "got {text:?}"
    );
}

/// The text spans of a one-run-per-string page, in both forms of `/DescendantFonts`.
fn spans_of(encoding: &str, w: &str, content: &[u8]) -> Vec<(bool, Vec<TextSpan>)> {
    [false, true]
        .into_iter()
        .map(|inline| {
            let pdf = type0_pdf(encoding, "Korea1", inline, w, content);
            let backend = RawBackend::load_bytes(&pdf).expect("parses");
            let spans = LayoutAnalyzer::new(&backend)
                .extract_page_spans(1)
                .expect("spans");
            (inline, spans)
        })
        .collect()
}

fn span<'a>(spans: &'a [TextSpan], text: &str, inline: bool) -> &'a TextSpan {
    spans
        .iter()
        .find(|s| s.text == text)
        .unwrap_or_else(|| panic!("inline={inline}: no {text:?} span in {spans:?}"))
}

/// Showing text advances the text position by the CIDFont's declared widths
/// (§9.7.4.3) — under a predefined CMap the code is first resolved to its CID. Two
/// strings shown back to back must therefore sit side by side, not on top of each other.
#[test]
fn a_run_under_a_unicode_cmap_advances_by_its_declared_widths() {
    // Adobe-Korea1 CIDs 34 and 35 are `A` and `B`; give them distinct widths so the
    // advance can only come out right through the code → CID step.
    let content = format!(
        "BT /F1 10 Tf 20 50 Td <{}> Tj <{}> Tj ET",
        hex(&utf16be("AB")),
        hex(&utf16be("한")),
    );
    for (inline, spans) in spans_of("UniKS-UCS2-H", "34[700 300]", content.as_bytes()) {
        let first = span(&spans, "AB", inline);
        let second = span(&spans, "한", inline);
        assert!(
            first.width_measured,
            "inline={inline}: AB's width must be measured"
        );
        assert!(
            (first.width - 10.0).abs() < 0.01,
            "inline={inline}: AB is 0.7 + 0.3 em, got {}",
            first.width
        );
        assert!(
            (second.x - 30.0).abs() < 0.01,
            "inline={inline}: the second run starts where AB ends, got x = {}",
            second.x
        );
        assert!(
            (second.width - 10.0).abs() < 0.01,
            "inline={inline}: 한 takes /DW, got {}",
            second.width
        );
    }
}

/// The same holds for a legacy CMap: `KSC-EUC-H` codes resolve to CIDs through the
/// CMap's own table before the widths are looked up.
#[test]
fn a_run_under_a_legacy_cmap_advances_by_its_declared_widths() {
    // EUC-KR: `A` is the one-byte code 0x41 — CID 8127, the half-width form this CMap
    // selects — and 한 is 0xC7D1.
    let content = b"BT /F1 10 Tf 20 50 Td <41> Tj <C7D1> Tj ET";
    for (inline, spans) in spans_of("KSC-EUC-H", "8127[700]", content) {
        let second = span(&spans, "한", inline);
        assert!(
            (second.x - 27.0).abs() < 0.01,
            "inline={inline}: the second run starts where `A` (0.7 em) ends, got x = {}",
            second.x
        );
    }
}

/// Under a vertical CMap a run advances down the page (§9.7.4.3): with no `/W2` each glyph
/// moves the text position by `/DW2`'s default `-1000`, and the run's horizontal extent is its
/// widest glyph, not the length of the column.
#[test]
fn a_run_under_a_vertical_cmap_advances_down_by_the_default_displacement() {
    let content = format!(
        "BT /F1 10 Tf 20 50 Td <{}> Tj <{}> Tj ET",
        hex(&utf16be("AB")),
        hex(&utf16be("한")),
    );
    for (inline, spans) in spans_of("UniKS-UCS2-V", "34[700 300]", content.as_bytes()) {
        let column = span(&spans, "AB", inline);
        assert!(column.width_measured, "inline={inline}");
        assert!(
            (column.width - 7.0).abs() < 0.01,
            "inline={inline}: the widest glyph is 0.7 em, got {}",
            column.width
        );
        let next = span(&spans, "한", inline);
        assert!(
            (column.y - next.y - 20.0).abs() < 0.01,
            "inline={inline}: two glyphs of 10 pt move 20 pt down, got {column:?} then {next:?}"
        );
        // Each glyph hangs from the column axis by half its own width (the default `vx`).
        assert!((column.x - 16.5).abs() < 0.01 && (next.x - 15.0).abs() < 0.01);
    }
}
