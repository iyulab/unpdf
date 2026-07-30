//! A PDF text string is either PDFDocEncoded or UTF-16BE, with or without the
//! byte-order mark the spec asks for. Form field strings were read one byte at a time
//! with no UTF-16 handling at all.
//!
//! The measured defect is the marked form, which is what AcroForm producers write:
//! `FE FF` is not valid UTF-8, so lossy decoding emitted two U+FFFD before the text. On
//! a real form every field name read
//! `\u{FFFD}\u{FFFD}topmostSubform[0].\u{FFFD}\u{FFFD}Page1[0]…`, one pair per segment.
//!
//! The unmarked form is handled only where it is unambiguous — every even-offset byte
//! zero, i.e. an all-ASCII string. Guessing more widely accepts `CHAP\0TER` and rewrites
//! it as CJK, so the mixed-script unmarked case stays on the single-byte reading and is
//! asserted as a limit rather than fixed.
//!
//! PDFs are assembled byte-by-byte rather than read from `test-files/` (gitignored, so
//! fixture-based tests silently skip in CI). The real-corpus test at the bottom guards
//! local runs, where the motivating documents are available.

use std::path::Path;
use unpdf::model::FieldValue;
use unpdf::PdfParser;

/// Assemble a structurally valid PDF from 1-indexed object bodies, computing a
/// traditional xref table over their real offsets.
fn assemble(objects: &[String], extra_trailer: &str) -> Vec<u8> {
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
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {} >>\n",
            objects.len() + 1,
            extra_trailer
        )
        .as_bytes(),
    );
    out.extend_from_slice(format!("startxref\n{}\n%%EOF", xref_offset).as_bytes());
    out
}

/// `s` as UTF-16BE inside a PDF hex string, so the exact bytes reach the parser.
fn hex_utf16be(s: &str, bom: bool) -> String {
    let mut hex = String::from("<");
    if bom {
        hex.push_str("FEFF");
    }
    for unit in s.encode_utf16() {
        hex.push_str(&format!("{:04X}", unit));
    }
    hex.push('>');
    hex
}

/// A one-page document with a single AcroForm text field whose name and value are the
/// literal PDF object strings given (so the caller controls the encoding on the wire).
fn form_pdf(name_obj: &str, value_obj: &str) -> Vec<u8> {
    assemble(
        &[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".to_string(),
            format!(
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T {} /V {} /Rect [0 0 100 20] >>",
                name_obj, value_obj
            ),
        ],
        "",
    )
}

fn fields(data: &[u8]) -> Vec<(String, Option<String>)> {
    let doc = PdfParser::from_bytes(data)
        .expect("fixture should load")
        .parse()
        .expect("fixture should parse");
    doc.form_fields
        .iter()
        .map(|f| {
            let value = match &f.value {
                Some(FieldValue::Text(t)) => Some(t.clone()),
                Some(FieldValue::Choice(t)) => Some(t.clone()),
                _ => None,
            };
            (f.name.clone(), value)
        })
        .collect()
}

#[test]
fn bomless_utf16be_field_name_is_decoded_not_merely_stripped() {
    let data = form_pdf(
        &hex_utf16be("topmostSubform", false),
        &hex_utf16be("value", false),
    );
    assert_eq!(
        fields(&data),
        vec![("topmostSubform".to_string(), Some("value".to_string()))]
    );
}

/// The regression that mattered, and the one the real corpus exhibits: a field string
/// carrying the byte-order mark. `FE FF` is not valid UTF-8, so the old reading emitted
/// two U+FFFD before the text — for an all-ASCII name that was the *only* damage, which
/// is why an ASCII-only assertion would not have caught it.
#[test]
fn bom_carrying_utf16be_field_strings_are_decoded() {
    let data = form_pdf(&hex_utf16be("성명", true), &hex_utf16be("홍길동", true));
    assert_eq!(
        fields(&data),
        vec![("성명".to_string(), Some("홍길동".to_string()))]
    );

    let ascii = form_pdf(&hex_utf16be("topmostSubform", true), "(v)");
    let got = fields(&ascii);
    assert_eq!(got[0].0, "topmostSubform");
    assert!(
        !got[0].0.contains('\u{FFFD}'),
        "byte-order mark leaked as U+FFFD: {:?}",
        got[0].0
    );
}

/// Documented limit rather than a fix: UTF-16BE with no mark whose even bytes are not
/// all zero keeps the single-byte reading. Detecting it would mean accepting
/// `CHAP\0TER` as UTF-16BE too — see `looks_like_bomless_utf16be`. Nothing regresses
/// here; this input was read the same way before.
#[test]
fn bomless_utf16be_mixing_ascii_and_non_ascii_is_left_alone() {
    let data = form_pdf(&hex_utf16be("이름(name)", false), "(v)");
    let got = fields(&data);
    assert_eq!(got.len(), 1);
    assert_ne!(got[0].0, "이름(name)", "detection would need the mark");
    // Whatever it becomes, it is still text: no NUL can reach the C ABI.
    assert!(!got[0].0.contains('\0'));

    // With the mark, the same name decodes.
    let marked = form_pdf(&hex_utf16be("이름(name)", true), "(v)");
    assert_eq!(fields(&marked)[0].0, "이름(name)");
}

/// A single-byte field name must survive untouched — the UTF-16BE path is a guess and
/// must not fire on ordinary strings.
#[test]
fn single_byte_field_strings_are_untouched() {
    let data = form_pdf("(FirstName)", "(John)");
    assert_eq!(
        fields(&data),
        vec![("FirstName".to_string(), Some("John".to_string()))]
    );
}

/// Document metadata already handled the marked form; the all-zero-high-byte form is
/// what it gained. Both are asserted so neither path silently changes.
#[test]
fn utf16be_metadata_titles_are_decoded_with_and_without_the_mark() {
    fn title_of(title_obj: &str) -> Option<String> {
        let data = assemble(
            &[
                "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
                format!("<< /Title {} >>", title_obj),
            ],
            "/Info 4 0 R",
        );
        PdfParser::from_bytes(&data)
            .expect("fixture should load")
            .parse()
            .expect("fixture should parse")
            .metadata
            .title
    }

    assert_eq!(
        title_of(&hex_utf16be("보고서 2026", true)).as_deref(),
        Some("보고서 2026")
    );
    assert_eq!(
        title_of(&hex_utf16be("Report 2026", false)).as_deref(),
        Some("Report 2026")
    );
}

/// The motivating documents. Field names must be text: no NUL (the C ABI cannot carry
/// one) and no U+FFFD (which is how the old reading reported anything non-ASCII).
/// Skips when `test-files/` is absent, so this guards local runs rather than CI.
#[test]
fn real_form_corpus_field_names_are_text() {
    let mut checked = 0usize;
    for rel in [
        "test-files/forms/pdf-form-sample.pdf",
        "test-files/forms/pdflatex-form.pdf",
    ] {
        let path = Path::new(rel);
        if !path.exists() {
            continue;
        }
        let doc = unpdf::parse_file(path).unwrap_or_else(|e| panic!("{rel}: {e}"));
        for field in &doc.form_fields {
            assert!(
                !field.name.contains('\0'),
                "{rel}: field name carries NUL: {:?}",
                field.name
            );
            assert!(
                !field.name.contains('\u{FFFD}'),
                "{rel}: field name degraded to U+FFFD: {:?}",
                field.name
            );
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping: no form fixtures present under test-files/");
    }
}
