//! End-to-end coverage for the embedded TrueType `cmap` extraction path
//! (`Type0`/Identity-H -> `FontFile2` -> `parse_truetype_cmap_table`).
//!
//! `src/parser/font.rs`'s own unit tests already prove `parse_truetype_cmap_table`
//! reverses a format-12 subtable correctly in isolation (cycle-33 — including the
//! fix that made a lone `(3,10)` subtable selectable at all). What they cannot prove
//! is that `backend.rs` actually wires a real PDF's `/FontFile2` stream into that
//! function: object resolution through `DescendantFonts` -> `FontDescriptor` ->
//! `FontFile2`, and the Identity-H content-stream bytes being read as raw GIDs. This
//! file assembles a complete, minimal PDF with a *real* embedded TrueType font
//! program (sfnt + `cmap` table, format 12, `(3,10)` — the pairing cycle-33 fixed)
//! to close that gap.
//!
//! PDFs are assembled byte-by-byte here, not read from `test-files/` (gitignored,
//! so fixture-based tests silently skip in CI) — same rationale as
//! `suppression_reporting_test.rs`.

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::PdfParser;

/// Assemble a structurally valid PDF from 1-indexed object bodies (raw bytes, so a
/// stream object can carry arbitrary binary content such as an embedded font
/// program), computing a traditional xref table over their real offsets.
fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());

    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
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

/// A minimal single-table sfnt font wrapping one format-12 `cmap` subtable at
/// `(platformID=3, encodingID=10)` — Windows full-repertoire (BMP + supplementary
/// planes), the standard real-world pairing for format 12. One group maps ASCII
/// 'A'..'C' to GIDs 5..7. The byte layout mirrors `src/parser/font.rs`'s own
/// `format12_subtable_ascii_and_supplementary_pua` / `wrap_cmap_subtables` test
/// helpers (duplicated here since this integration test compiles against the
/// public API only and cannot reach those `pub(crate)` test helpers).
fn embedded_truetype_font_format12_only() -> Vec<u8> {
    // -- format 12 subtable --
    let mut subtable = Vec::new();
    subtable.extend_from_slice(&12u16.to_be_bytes()); // format
    subtable.extend_from_slice(&0u16.to_be_bytes()); // reserved
    subtable.extend_from_slice(&0u32.to_be_bytes()); // length (unused by parser)
    subtable.extend_from_slice(&0u32.to_be_bytes()); // language (unused)
    subtable.extend_from_slice(&1u32.to_be_bytes()); // nGroups
    subtable.extend_from_slice(&0x0041u32.to_be_bytes()); // startCharCode 'A'
    subtable.extend_from_slice(&0x0043u32.to_be_bytes()); // endCharCode 'C'
    subtable.extend_from_slice(&5u32.to_be_bytes()); // startGlyphID

    // -- cmap table: header + one (3,10) subtable record --
    let mut cmap_table = Vec::new();
    cmap_table.extend_from_slice(&0u16.to_be_bytes()); // version
    cmap_table.extend_from_slice(&1u16.to_be_bytes()); // numTables
    cmap_table.extend_from_slice(&3u16.to_be_bytes()); // platformID (Windows)
    cmap_table.extend_from_slice(&10u16.to_be_bytes()); // encodingID (full repertoire)
    cmap_table.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable (4 + 8)
    cmap_table.extend_from_slice(&subtable);

    // -- sfnt wrapper: header + one 'cmap' table-directory record --
    let cmap_offset: u32 = 12 + 16; // sfnt header + one directory record
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // sfnt version
    font.extend_from_slice(&1u16.to_be_bytes()); // numTables
    font.extend_from_slice(&0u16.to_be_bytes()); // searchRange (unused)
    font.extend_from_slice(&0u16.to_be_bytes()); // entrySelector (unused)
    font.extend_from_slice(&0u16.to_be_bytes()); // rangeShift (unused)
    font.extend_from_slice(b"cmap");
    font.extend_from_slice(&0u32.to_be_bytes()); // checksum (unused)
    font.extend_from_slice(&cmap_offset.to_be_bytes());
    font.extend_from_slice(&(cmap_table.len() as u32).to_be_bytes());
    font.extend_from_slice(&cmap_table);
    font
}

/// A one-page PDF whose Identity-H content stream draws GIDs 5, 6, 7 (raw 2-byte
/// codes, no ToUnicode) through a Type0/CIDFontType2 font backed by a real embedded
/// `FontFile2` program — the full real-world object graph
/// `Type0 -> DescendantFonts -> CIDFontType2 -> FontDescriptor -> FontFile2`.
fn embedded_format12_cmap_pdf() -> Vec<u8> {
    let font_program = embedded_truetype_font_format12_only();
    let mut content = b"BT /F1 12 Tf 72 720 Td (".to_vec();
    content.extend_from_slice(&[0x00, 0x05, 0x00, 0x06, 0x00, 0x07]); // GIDs 5, 6, 7
    content.extend_from_slice(b") Tj ET");

    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content,
            b"\nendstream".to_vec(),
        ]
        .concat(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /Synthetic /Encoding /Identity-H \
          /DescendantFonts [6 0 R] >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Synthetic \
          /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
          /FontDescriptor 7 0 R >>"
            .to_vec(),
        b"<< /Type /FontDescriptor /FontName /Synthetic /Flags 4 /FontFile2 8 0 R >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", font_program.len()).into_bytes(),
            font_program,
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];

    assemble(&objects)
}

#[test]
fn embedded_truetype_format12_cmap_is_wired_through_the_real_pdf_object_graph() {
    let doc = PdfParser::from_bytes(&embedded_format12_cmap_pdf())
        .and_then(|p| p.parse())
        .expect("fixture must parse");

    let text = to_markdown(&doc, &RenderOptions::default()).expect("markdown render");
    assert!(
        text.contains("ABC"),
        "expected the embedded (3,10) format-12 cmap to resolve GIDs 5/6/7 to \
         'A'/'B'/'C' through the real Type0 -> DescendantFonts -> FontDescriptor -> \
         FontFile2 object graph, got: {text:?}"
    );
    assert_eq!(
        doc.extraction_quality.suppressed_text_runs, 0,
        "a resolvable embedded cmap must not report suppression"
    );
}
