//! CID-to-Unicode lookup tables for Adobe CJK character collections.
//!
//! Tables are generated at compile time from Adobe's cid2code.txt files.

// Include the generated tables
include!(concat!(env!("OUT_DIR"), "/cmap_tables.rs"));

/// Look up a CID in the specified Adobe character collection.
pub fn lookup_cid(registry: &str, ordering: &str, cid: u32) -> Option<char> {
    if registry != "Adobe" {
        return None;
    }

    let table: &[(u32, u32)] = match ordering {
        o if o.starts_with("Korea1") => CID_TO_UNICODE_KOREA1,
        o if o.starts_with("Japan1") => CID_TO_UNICODE_JAPAN1,
        o if o.starts_with("CNS1") => CID_TO_UNICODE_CNS1,
        o if o.starts_with("GB1") => CID_TO_UNICODE_GB1,
        _ => return None,
    };

    // Binary search on sorted table
    table
        .binary_search_by_key(&cid, |&(c, _)| c)
        .ok()
        .and_then(|idx| char::from_u32(table[idx].1))
}

/// Decode a byte sequence using CIDSystemInfo-based CMap lookup.
/// For Identity-H/V encoding, each 2-byte pair is treated as a CID.
pub fn decode_with_cid_system_info(
    registry: &str,
    ordering: &str,
    bytes: &[u8],
) -> Option<String> {
    if bytes.len() < 2 || bytes.len() % 2 != 0 {
        return None;
    }

    let mut result = String::new();
    let mut any_mapped = false;

    for chunk in bytes.chunks(2) {
        let cid = ((chunk[0] as u32) << 8) | (chunk[1] as u32);
        if let Some(ch) = lookup_cid(registry, ordering, cid) {
            result.push(ch);
            any_mapped = true;
        }
    }

    if any_mapped {
        Some(result)
    } else {
        None
    }
}
