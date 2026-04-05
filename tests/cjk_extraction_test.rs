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
