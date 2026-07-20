use unpdf::parser::cmap_table;

#[test]
fn test_korea1_lookup_space() {
    // CID 1 in Adobe-Korea1 maps to U+00A0 (or U+0020)
    let result = cmap_table::lookup_cid("Adobe", "Korea1", 1);
    assert!(result.is_some(), "CID 1 should map to a character");
}

#[test]
fn test_korea1_lookup_hangul() {
    // CID 1086 in Adobe-Korea1 maps to U+AC00 (가)
    let result = cmap_table::lookup_cid("Adobe", "Korea1", 1086);
    assert_eq!(result, Some('가'), "CID 1086 should map to 가 (U+AC00)");
}

#[test]
fn test_japan1_lookup() {
    let result = cmap_table::lookup_cid("Adobe", "Japan1", 1);
    assert!(result.is_some(), "Japan1 CID 1 should map");
}

#[test]
fn test_unknown_collection() {
    let result = cmap_table::lookup_cid("Adobe", "Unknown", 1);
    assert_eq!(result, None);
}

#[test]
fn test_unknown_registry() {
    let result = cmap_table::lookup_cid("NotAdobe", "Korea1", 1);
    assert_eq!(result, None);
}

#[test]
fn test_cid_out_of_range() {
    let result = cmap_table::lookup_cid("Adobe", "Korea1", 999999);
    assert_eq!(result, None);
}

#[test]
fn test_decode_bytes() {
    // CID 2 = 0x0002, should map to U+0021 ('!')
    let bytes = [0x00, 0x02];
    let result = cmap_table::decode_with_cid_system_info("Adobe", "Korea1", &bytes);
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "!");
}

#[test]
fn test_decode_empty() {
    let result = cmap_table::decode_with_cid_system_info("Adobe", "Korea1", &[]);
    assert_eq!(result, None);
}

#[test]
fn test_decode_odd_bytes() {
    let result = cmap_table::decode_with_cid_system_info("Adobe", "Korea1", &[0x00]);
    assert_eq!(result, None);
}

use std::path::Path;
use unpdf::parse_file;

#[test]
fn test_korean_pdf_extracts_text() {
    let path = Path::new("test-files/cjk/korean-test.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    let fffd_count = text.chars().filter(|&c| c == '\u{FFFD}').count();
    let total_chars = text.chars().count();
    if total_chars > 0 {
        let fffd_ratio = fffd_count as f32 / total_chars as f32;
        assert!(
            fffd_ratio < 0.3,
            "Too many replacement characters: {:.1}% ({}/{})",
            fffd_ratio * 100.0,
            fffd_count,
            total_chars
        );
    }
}

#[test]
fn test_korean_pdf_no_replacement_flood() {
    let path = Path::new("test-files/cjk/korean-test.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    let text = doc.plain_text();
    // Count max consecutive U+FFFD chars
    let max_consecutive_fffd = text
        .chars()
        .fold((0u32, 0u32), |(max, current), c| {
            if c == '\u{FFFD}' {
                (max.max(current + 1), current + 1)
            } else {
                (max, 0)
            }
        })
        .0;
    assert!(
        max_consecutive_fffd < 5,
        "Found {} consecutive replacement characters",
        max_consecutive_fffd
    );
}

#[test]
fn test_extraction_quality_korean() {
    let path = Path::new("test-files/cjk/korean-test.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    // Just report metrics, don't assert is_good() since it depends on test PDF content
    println!(
        "Korean PDF quality: chars={}, words={}, fffd={}, ratio={:.2}%",
        doc.extraction_quality.char_count,
        doc.extraction_quality.word_count,
        doc.extraction_quality.replacement_char_count,
        doc.extraction_quality.replacement_char_ratio() * 100.0,
    );
}

#[test]
fn test_cjk_table_no_oversplit() {
    let path = Path::new("test-files/cjk/korean-test.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    for page in &doc.pages {
        for block in &page.elements {
            if let unpdf::model::Block::Table(table) = block {
                for row in &table.rows {
                    assert!(
                        row.cells.len() <= 10,
                        "Table row has {} cells — likely CJK oversplit",
                        row.cells.len()
                    );
                }
            }
        }
    }
}

/// A Type0 font with the predefined `KSC-EUC-H` CMap and no ToUnicode / no embedded
/// font file decodes through the CMap: the codes are EUC-KR.
#[test]
fn test_type0_ksc_euc_h_decodes_korean() {
    let doc = unpdf::parse_bytes(&predefined_cmap_pdf("KSC-EUC-H")).unwrap();
    assert_eq!(doc.plain_text().trim(), "검야ㅓ");
}

/// A predefined CMap outside the supported set must not fall back to byte-wise
/// Latin-1 decoding — that produced mojibake like `°Ë ,¥õ ²ô`. Emitting nothing is
/// the correct behaviour for a composite font with no usable CMap.
#[test]
fn test_type0_unsupported_cmap_no_mojibake() {
    let doc = unpdf::parse_bytes(&predefined_cmap_pdf("KSC-Johab-H")).unwrap();
    let text = doc.plain_text();
    assert!(
        text.trim().is_empty(),
        "Type0 font without usable CMap must yield no text, got {text:?}"
    );
}

/// Minimal PDF whose only font is a Type0/CIDFontType2 font using a predefined CMap,
/// with no ToUnicode and no embedded font file — the structure emitted by scanner
/// OCR layers (e.g. Canon SC1011). The text bytes are `검야ㅓ` in EUC-KR.
fn predefined_cmap_pdf(encoding: &str) -> Vec<u8> {
    let content = b"BT /F1 12 Tf 20 50 Td <B0CBBEDFA4C3> Tj ET\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        format!(
            "<</Length {}>>\nstream\n{}\nendstream",
            content.len(),
            String::from_utf8_lossy(content)
        )
        .into_bytes(),
        format!(
            "<</Type/Font/Subtype/Type0/BaseFont/Dotum\
             /DescendantFonts[6 0 R]/Encoding/{encoding}>>"
        )
        .into_bytes(),
        b"<</Type/Font/Subtype/CIDFontType2/BaseFont/Dotum\
          /CIDSystemInfo<</Registry(Adobe)/Ordering(Korea1)/Supplement 2>>\
          /FontDescriptor 7 0 R/DW 1000>>"
            .to_vec(),
        b"<</Type/FontDescriptor/FontName/Dotum/Flags 39\
          /FontBBox[-150 -136 1100 864]/ItalicAngle 0/Ascent 864/Descent -136\
          /CapHeight 864/StemV 91>>"
            .to_vec(),
    ];

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
