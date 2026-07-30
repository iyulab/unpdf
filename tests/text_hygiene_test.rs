//! Extracted text must be text: no C0/C1 control characters but `\n`, `\r`, `\t`.
//!
//! PDF string literals may legally hold control bytes (`\000` is a valid octal
//! escape) and some producers leave NUL padding in the text layer. Such a file is
//! not damaged, so accepting it is right — but reporting the control byte back as
//! text corrupts every output format, and at the C ABI it cannot be transported at
//! all: `CString::new` fails and the caller loses the entire page or document.
//!
//! The Rust-side suite could never have caught this on its own: the Rust API hands
//! back a `String` containing the NUL without complaint. Only a test that pushes the
//! text through the ABI observes the loss — hence the `ffi` tests at the bottom.
//!
//! PDFs are assembled byte-by-byte here rather than read from `test-files/`
//! (gitignored, so fixture-based tests silently skip in CI).

use unpdf::render::{to_json, to_markdown, JsonFormat, RenderOptions};
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

/// A one-page document whose page text is `literal`, written as a PDF string literal
/// (so `\\000` in the caller's string reaches the parser as an octal escape).
fn page_text_pdf(literal: &str) -> Vec<u8> {
    let body = format!("BT /F1 12 Tf 72 720 Td ({}) Tj ET", literal);
    assemble(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
                .to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", body.len(), body),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ],
        "",
    )
}

/// A document carrying a NUL in its `/Info` title and author and in an outline title.
fn metadata_pdf() -> Vec<u8> {
    let body = "BT /F1 12 Tf 72 720 Td (plain) Tj ET";
    assemble(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
                .to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", body.len(), body),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            "<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>".to_string(),
            "<< /Title (CHAP\\000TER) /Parent 6 0 R >>".to_string(),
            "<< /Title (DOC\\000TITLE) /Author (A\\000B) >>".to_string(),
        ],
        "/Info 8 0 R",
    )
}

fn parse(data: &[u8]) -> unpdf::Document {
    PdfParser::from_bytes(data)
        .and_then(|p| p.parse())
        .expect("document should parse — a legal control byte is not damage")
}

fn markdown(doc: &unpdf::Document) -> String {
    to_markdown(doc, &RenderOptions::default()).expect("markdown render")
}

/// Every C0/C1 control character, except the whitespace text legitimately uses.
fn non_text_controls(text: &str) -> Vec<char> {
    text.chars()
        .filter(|&c| {
            !matches!(c, '\n' | '\r' | '\t')
                && (c <= '\u{1F}' || c == '\u{7F}' || ('\u{80}'..='\u{9F}').contains(&c))
        })
        .collect()
}

#[test]
fn nul_in_page_text_is_removed_not_carried_into_the_output() {
    let doc = parse(&page_text_pdf(r"HELLO\000WORLD"));

    let text = doc.plain_text();
    assert!(
        non_text_controls(&text).is_empty(),
        "page text still carries control characters: {:?}",
        non_text_controls(&text)
    );
    assert!(
        text.contains("HELLOWORLD"),
        "the readable characters must survive; got {:?}",
        text
    );
}

#[test]
fn nul_bearing_page_still_yields_its_text_rather_than_failing() {
    // The document must remain fully usable — the point is not merely "no NUL" but
    // "the page is not lost", which is what a consumer experienced instead.
    let doc = parse(&page_text_pdf(r"HELLO\000WORLD"));
    assert_eq!(doc.metadata.page_count, 1);
    assert!(!doc.plain_text().trim().is_empty());
}

#[test]
fn markdown_and_json_outputs_are_free_of_control_characters() {
    let doc = parse(&page_text_pdf(r"A\000B\001C"));

    let rendered = markdown(&doc);
    assert!(
        non_text_controls(&rendered).is_empty(),
        "markdown output is not a clean text file: {:?}",
        non_text_controls(&rendered)
    );

    let json = to_json(&doc, JsonFormat::Compact).expect("json serialization");
    assert!(
        non_text_controls(&json).is_empty(),
        "json output embeds control characters: {:?}",
        non_text_controls(&json)
    );
}

#[test]
fn metadata_and_outline_titles_are_sanitized_not_dropped() {
    let doc = parse(&metadata_pdf());

    // The failure this replaces was silent *absence*: a title that cannot cross the
    // ABI is reported as "no title", indistinguishable from a document without one.
    assert_eq!(doc.metadata.title.as_deref(), Some("DOCTITLE"));
    assert_eq!(doc.metadata.author.as_deref(), Some("AB"));

    let outline = doc.outline.as_ref().expect("outline present");
    assert_eq!(outline.items[0].title, "CHAPTER");
}

#[test]
fn suspect_decode_is_still_suppressed_wholesale() {
    // Control-character density is how a mis-decoded run is recognised (a CID font
    // read as Latin-1). Sanitising must not run before that judgement, or this text
    // would drop under the threshold and be emitted as garbage-derived letters.
    let doc = parse(&page_text_pdf(r"A\001B\014C\033D"));
    let text = doc.plain_text();

    assert!(
        non_text_controls(&text).is_empty(),
        "control characters leaked: {:?}",
        non_text_controls(&text)
    );
    assert!(
        !text.contains("ABCD"),
        "a run judged as a bad decode must stay suppressed, not be cleaned up into \
         plausible-looking text; got {:?}",
        text
    );
}

#[test]
fn ordinary_text_is_untouched() {
    let doc = parse(&page_text_pdf("Hello World"));
    assert!(doc.plain_text().contains("Hello World"));
}

/// No real document should contain control characters, so the invariant must hold
/// across the whole corpus with nothing removed. Skips when `test-files/` is absent
/// (it is gitignored), so this guards local runs rather than CI.
#[test]
fn real_corpus_extracts_clean_text() {
    let root = std::path::Path::new("test-files");
    if !root.exists() {
        eprintln!("skipping: test-files/ not present");
        return;
    }

    let mut checked = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("pdf") {
                continue;
            }
            let Ok(doc) = PdfParser::open(&path).and_then(|p| p.parse()) else {
                continue;
            };

            for (surface, text) in [
                ("markdown", markdown(&doc)),
                ("plain text", doc.plain_text()),
            ] {
                let found = non_text_controls(&text);
                assert!(
                    found.is_empty(),
                    "{} of {} carries {} control character(s): {:?}",
                    surface,
                    path.display(),
                    found.len(),
                    &found[..found.len().min(8)]
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "test-files/ exists but held no parsable PDF");
}

// ---------------------------------------------------------------------------
// The ABI is where the loss was actually observed.
// ---------------------------------------------------------------------------

#[cfg(feature = "ffi")]
mod ffi {
    use std::ffi::CStr;

    use unpdf::ffi::{
        unpdf_free_document, unpdf_free_string, unpdf_get_title, unpdf_last_error_kind,
        unpdf_parse_bytes, unpdf_plain_text, unpdf_to_markdown, UNPDF_ERROR_NONE,
    };

    /// Take ownership of an FFI string, asserting it was produced at all.
    unsafe fn expect_string(ptr: *mut std::os::raw::c_char, what: &str) -> String {
        assert!(
            !ptr.is_null(),
            "{} could not cross the ABI (error kind {})",
            what,
            unpdf_last_error_kind()
        );
        let owned = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        unpdf_free_string(ptr);
        owned
    }

    #[test]
    fn nul_bearing_document_crosses_the_abi_intact() {
        let data = super::page_text_pdf(r"HELLO\000WORLD");
        unsafe {
            let doc = unpdf_parse_bytes(data.as_ptr(), data.len());
            assert!(!doc.is_null(), "document should parse");

            let text = expect_string(unpdf_plain_text(doc), "plain text");
            assert!(text.contains("HELLOWORLD"), "got {:?}", text);

            let markdown = expect_string(unpdf_to_markdown(doc, 0), "markdown");
            assert!(markdown.contains("HELLOWORLD"), "got {:?}", markdown);

            assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_NONE);
            unpdf_free_document(doc);
        }
    }

    #[test]
    fn nul_bearing_title_is_reported_rather_than_appearing_absent() {
        let data = super::metadata_pdf();
        unsafe {
            let doc = unpdf_parse_bytes(data.as_ptr(), data.len());
            assert!(!doc.is_null());

            // A null return here means "no title" — the silent failure mode this guards.
            let title = expect_string(unpdf_get_title(doc), "title");
            assert_eq!(title, "DOCTITLE");

            unpdf_free_document(doc);
        }
    }
}
