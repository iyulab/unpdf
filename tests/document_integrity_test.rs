//! Structural-integrity reporting: a damaged document must not lose pages silently.
//!
//! The PDFs here are assembled byte-by-byte inside the test rather than read from
//! `test-files/` (gitignored, so fixture-based tests silently skip in CI). Everything
//! these tests assert is therefore reproducible from the repository alone.

use unpdf::parser::raw::RawDocument;
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

fn content_stream(text: &str) -> String {
    let body = format!("BT /F1 12 Tf 72 720 Td ({}) Tj ET", text);
    format!("<< /Length {} >>\nstream\n{}\nendstream", body.len(), body)
}

/// A two-page document. Page 2's node is object 5, which the damage tests overwrite.
fn two_page_pdf() -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 7 0 R >> >> /Contents {} 0 R >>",
            contents
        )
    };
    assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_string(),
        page(4),
        content_stream("Page one"),
        page(6),
        content_stream("Page two"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ])
}

fn find(data: &[u8], needle: &[u8], from: usize) -> usize {
    data[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
        .unwrap_or_else(|| panic!("{:?} not found", String::from_utf8_lossy(needle)))
}

/// Overwrite an object's bytes in place, leaving the xref offset pointing at rubble —
/// the shape real damage takes when a region of the file is zeroed.
fn zero_out_object(data: &mut [u8], obj_header: &str) {
    let start = find(data, obj_header.as_bytes(), 0);
    let end = (start + 48).min(data.len());
    for byte in &mut data[start..end] {
        *byte = 0;
    }
}

/// Point an object's xref offset past the end of the file — the shape damage takes when
/// a file is truncated: the table survives, the object it names does not.
fn break_xref_offset(data: &mut [u8], obj_header: &str) {
    let obj_offset = find(data, obj_header.as_bytes(), 0);
    let xref = find(data, b"xref\n", 0);
    let entry = find(data, format!("{:010}", obj_offset).as_bytes(), xref);
    data[entry..entry + 10].copy_from_slice(b"9999999999");
}

#[test]
fn intact_document_reports_no_damage() {
    let data = two_page_pdf();

    let scan = RawDocument::load(&data).unwrap().scan_page_tree();
    assert_eq!(scan.pages.len(), 2);
    assert_eq!(scan.unresolved_nodes, 0);

    let doc = PdfParser::from_bytes(&data).unwrap().parse().unwrap();
    let q = &doc.extraction_quality;
    assert_eq!(doc.page_count(), 2);
    assert!(!q.pages_incomplete, "intact document must not claim damage");
    assert_eq!(q.declared_page_count, Some(2));
    assert_eq!(q.unresolved_page_nodes, 0);
    assert_eq!(q.skipped_object_count, 0);
    assert_eq!(
        q.warning_message(),
        None,
        "intact document must not warn: {:?}",
        q.warning_message()
    );
}

#[test]
fn lost_page_node_is_reported_not_swallowed() {
    let mut data = two_page_pdf();
    zero_out_object(&mut data, "5 0 obj");

    let scan = RawDocument::load(&data).unwrap().scan_page_tree();
    assert_eq!(scan.pages.len(), 1, "page 2's node is unreadable");
    assert!(scan.unresolved_nodes >= 1);

    let doc = PdfParser::from_bytes(&data).unwrap().parse().unwrap();
    let q = &doc.extraction_quality;

    // The regression this guards: parsing succeeds over a short page set, and without
    // these signals the caller cannot tell that apart from a genuine one-page document.
    assert_eq!(doc.page_count(), 1);
    assert!(q.pages_incomplete, "page loss must be observable");
    assert_eq!(q.declared_page_count, Some(2));
    assert!(q.unresolved_page_nodes >= 1);

    let warning = q.warning_message().expect("page loss must warn");
    assert!(
        warning.contains("missing from the output"),
        "warning should name the loss: {warning}"
    );
}

#[test]
fn declared_count_catches_loss_that_leaves_the_tree_walkable() {
    // `/Count` says 2, but the tree only offers one kid — the loss a node-level counter
    // cannot see, because nothing in the structure failed to resolve.
    let data = assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        content_stream("Page one"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ]);

    let scan = RawDocument::load(&data).unwrap().scan_page_tree();
    assert_eq!(scan.unresolved_nodes, 0, "nothing failed to resolve");

    let q = PdfParser::from_bytes(&data)
        .unwrap()
        .parse()
        .unwrap()
        .extraction_quality;
    assert!(
        q.pages_incomplete,
        "declared 2 pages but found 1 — must be reported"
    );
    assert_eq!(q.declared_page_count, Some(2));
}

#[test]
fn understated_count_is_not_treated_as_damage() {
    // Some writers understate `/Count`. More pages than declared is not a loss, and
    // flagging it would put a warning on intact documents.
    let data = assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        content_stream("Page one"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>"
            .to_string(),
        content_stream("Page two"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ]);

    let q = PdfParser::from_bytes(&data)
        .unwrap()
        .parse()
        .unwrap()
        .extraction_quality;
    assert!(!q.pages_incomplete);
    assert_eq!(q.warning_message(), None);
}

#[test]
fn unreadable_catalog_reports_damage_rather_than_an_empty_document() {
    let mut data = two_page_pdf();
    zero_out_object(&mut data, "1 0 obj");

    let q = PdfParser::from_bytes(&data)
        .unwrap()
        .parse()
        .unwrap()
        .extraction_quality;

    // Zero pages extracted "successfully" is the worst case: without a signal it reads
    // as an empty document rather than a document that could not be read.
    assert!(q.pages_incomplete, "total structural loss must be visible");
    assert!(q.warning_message().is_some());
}

/// Every real PDF in `test-files/` must report itself intact.
///
/// The damage signals are only worth having if they stay silent on healthy documents:
/// a `/Count` that disagrees with the page tree, or a page-tree node shape the walk
/// does not recognise, would turn every normal file into a false warning. Skips when
/// `test-files/` is absent (it is gitignored), so this guards local runs, not CI.
#[test]
fn real_fixtures_report_no_damage() {
    fn collect_pdfs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_pdfs(&path, out);
            } else if path.extension().is_some_and(|e| e == "pdf") {
                out.push(path);
            }
        }
    }

    let root = std::path::Path::new("test-files");
    if !root.exists() {
        eprintln!("skipping: test-files/ not present");
        return;
    }

    let mut pdfs = Vec::new();
    collect_pdfs(root, &mut pdfs);
    assert!(!pdfs.is_empty(), "test-files/ present but holds no PDFs");

    let mut checked = 0usize;
    for path in &pdfs {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        // Documents that fail to load are a different matter — this test is about files
        // that parse fine being wrongly accused of damage.
        let Ok(doc) = RawDocument::load(&data) else {
            continue;
        };
        let scan = doc.scan_page_tree();
        let declared = doc.declared_page_count();
        checked += 1;

        assert_eq!(
            scan.unresolved_nodes,
            0,
            "{} is intact but reported {} unresolved page-tree node(s)",
            path.display(),
            scan.unresolved_nodes
        );
        assert_eq!(
            declared,
            Some(scan.pages.len() as u32),
            "{}: declared /Count and pages found disagree",
            path.display()
        );
    }
    assert!(checked > 0, "no fixture could be loaded");
    eprintln!("checked {checked} of {} fixtures", pdfs.len());
}

#[test]
fn cyclic_page_tree_terminates() {
    // A `Pages` node listing itself as a kid. The walk must finish rather than recurse
    // until the stack gives out.
    let data = assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 2 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        content_stream("Page one"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ]);

    let scan = RawDocument::load(&data).unwrap().scan_page_tree();
    assert_eq!(scan.pages.len(), 1, "the one real page is still found");
    assert_eq!(
        scan.unresolved_nodes, 0,
        "revisiting a node is not a page loss"
    );
}

#[test]
fn skipped_objects_alone_do_not_claim_page_loss() {
    // Damaging a font object costs no page. `skipped_object_count` should notice it while
    // `pages_incomplete` stays false — the two signals must not be conflated.
    let mut data = two_page_pdf();
    break_xref_offset(&mut data, "7 0 obj");

    let doc = RawDocument::load(&data).unwrap();
    assert_eq!(doc.skipped_object_count(), 1);

    let q = PdfParser::from_bytes(&data)
        .unwrap()
        .parse()
        .unwrap()
        .extraction_quality;
    assert_eq!(q.skipped_object_count, 1);
    assert!(
        !q.pages_incomplete,
        "a lost font is not a lost page — must not warn about missing pages"
    );
}
