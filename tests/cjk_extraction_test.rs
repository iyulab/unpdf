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
