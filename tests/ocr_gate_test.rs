//! End-to-end behaviour of the low-confidence OCR text layer gate.
//!
//! The fixtures reproduce the structure a scanner emits: the page drawn as one
//! full-size image, with the OCR result on top in text rendering mode 3.

use unpdf::{parse_bytes, parse_bytes_with_options, ParseOptions};

/// Isolated jamo, Greek letters and maths symbols in EUC-KR — what an OCR engine
/// emits when it recognises strokes but no characters. Each code is shown by its own
/// `Tj`, as scanner OCR layers position every glyph individually.
const GARBAGE_CODES: &[&str] = &[
    "A4C3", "A4A1", "A4A2", "A4B2", "A5F5", "A5D5", "A1EF", "A1F9", "A5F2", "A5F4", "A1C6", "A1BE",
    "A1BF", "A1C0", "A1C1", "A1C2", "A1D2", "A1C5", "A5E1", "A5E2", "A5E3", "A4B1", "A4BD",
];

/// `한글 문서 처리 결과 확인` in EUC-KR — ordinary Korean prose.
const KOREAN_CODES: &[&str] = &[
    "C7D1B1DB", "B9AEBCAD", "C3B3B8AE", "B0E1B0FA", "C8AEC0CE", "C7D1B1DB", "B9AEBCAD", "C3B3B8AE",
    "B0E1B0FA", "C8AEC0CE",
];

#[test]
fn drops_unreadable_ocr_layer_over_scan() {
    let doc = parse_bytes(&scan_pdf(GARBAGE_CODES, INVISIBLE)).unwrap();
    assert_eq!(doc.plain_text().trim(), "");
    assert_eq!(doc.extraction_quality.suppressed_ocr_pages, 1);
    assert!(doc.pages[0].ocr_text_suppressed);
}

#[test]
fn keeps_readable_ocr_layer_over_scan() {
    let doc = parse_bytes(&scan_pdf(KOREAN_CODES, INVISIBLE)).unwrap();
    assert!(
        doc.plain_text().contains("한글"),
        "readable OCR text must survive, got {:?}",
        doc.plain_text()
    );
    assert_eq!(doc.extraction_quality.suppressed_ocr_pages, 0);
}

/// Visible text is never touched, however little sense it makes — only an
/// invisible layer over a scan is a candidate.
#[test]
fn keeps_visible_text() {
    let doc = parse_bytes(&scan_pdf(GARBAGE_CODES, VISIBLE)).unwrap();
    assert!(!doc.plain_text().trim().is_empty());
    assert_eq!(doc.extraction_quality.suppressed_ocr_pages, 0);
}

#[test]
fn opt_out_keeps_unreadable_layer() {
    let options = ParseOptions::new().with_ocr_suppression(false);
    let doc = parse_bytes_with_options(&scan_pdf(GARBAGE_CODES, INVISIBLE), options).unwrap();
    assert!(!doc.plain_text().trim().is_empty());
    assert_eq!(doc.extraction_quality.suppressed_ocr_pages, 0);
}

const INVISIBLE: u8 = 3;
const VISIBLE: u8 = 0;

/// A one-page PDF drawing `image` across the whole page with a text layer on top.
///
/// The text is a Type0 font using the predefined `KSC-EUC-H` CMap, as scanner OCR
/// layers do. Each entry of `codes` is EUC-KR encoded text shown by its own `Tj` at
/// its own position; enough are emitted to clear the classifier's minimum length.
fn scan_pdf(codes: &[&str], render_mode: u8) -> Vec<u8> {
    let show = codes
        .iter()
        .cycle()
        .take(codes.len() * 3)
        .enumerate()
        .map(|(i, hex)| {
            format!(
                "1 0 0 1 {} {} Tm <{hex}> Tj",
                20 + (i % 20) * 25,
                800 - (i / 20) * 20
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content =
        format!("q 595 0 0 842 0 0 cm /Im0 Do Q\nBT {render_mode} Tr /F1 10 Tf\n{show}\nET\n");

    // A 1×1 grey image is enough: only the CTM it is drawn with matters here.
    let image_data = [0x80u8];
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>/XObject<</Im0 8 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ),
        b"<</Type/Font/Subtype/Type0/BaseFont/Dotum\
          /DescendantFonts[6 0 R]/Encoding/KSC-EUC-H>>"
            .to_vec(),
        b"<</Type/Font/Subtype/CIDFontType2/BaseFont/Dotum\
          /CIDSystemInfo<</Registry(Adobe)/Ordering(Korea1)/Supplement 2>>\
          /FontDescriptor 7 0 R/DW 1000>>"
            .to_vec(),
        b"<</Type/FontDescriptor/FontName/Dotum/Flags 39\
          /FontBBox[-150 -136 1100 864]/ItalicAngle 0/Ascent 864/Descent -136\
          /CapHeight 864/StemV 91>>"
            .to_vec(),
        stream_object(
            "<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray\
              /BitsPerComponent 8/Length 1>>",
            &image_data,
        ),
    ];

    assemble(objects)
}

fn stream_object(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut obj = dict.as_bytes().to_vec();
    obj.extend_from_slice(b"\nstream\n");
    obj.extend_from_slice(data);
    obj.extend_from_slice(b"\nendstream");
    obj
}

fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
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
