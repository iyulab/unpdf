//! Predefined CJK CMap decoding for Type0 fonts.
//!
//! A composite font may reference one of Adobe's predefined CMaps by name
//! (`/Encoding /KSC-EUC-H`) instead of embedding a ToUnicode CMap. Those names
//! describe a legacy character encoding (EUC-KR, Shift-JIS, GBK, Big5) or a
//! Unicode encoding, plus a writing mode:
//!
//! ```text
//! KSC-EUC-H  →  KS X 1001 charset, EUC-KR encoding, horizontal
//! UniKS-UCS2-V  →  UCS-2 (UTF-16BE), vertical
//! ```
//!
//! Legacy CMaps map character codes to CIDs, which the character collection's
//! CID→Unicode table then resolves. Unicode CMaps encode the code points directly,
//! so no table is needed. Writing mode does not affect the mapping, only glyph
//! selection, so `-H` and `-V` share a table.
//!
//! CMaps outside the shipped set decode to `None`, which the caller treats the
//! same as any other unusable CMap — no text rather than mojibake.

use super::cmap_table::{lookup_cid, PredefinedCmap, PREDEFINED_CMAPS};

/// Decode a string from a content stream using a predefined CMap.
///
/// `encoding_name` is the font's `/Encoding` name, `registry`/`ordering` come from
/// the descendant CIDFont's `/CIDSystemInfo`. Returns `None` when the CMap is not
/// supported or nothing in `bytes` could be mapped.
pub fn decode(encoding_name: &str, registry: &str, ordering: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }

    let base = base_cmap_name(encoding_name);

    if is_unicode_cmap(base) {
        return decode_utf16be(bytes);
    }

    let cmap = find_table(ordering, base)?;
    decode_with_table(bytes, cmap, registry, ordering)
}

/// Strip the writing-mode suffix, yielding the `cid2code.txt` column name.
///
/// `KSC-EUC-H` → `KSC-EUC`, `UniJIS-UCS2-HW-V` → `UniJIS-UCS2-HW`. The Adobe-Japan1
/// ISO-2022-JP CMaps are named just `H` and `V`, and both use the `H` column.
fn base_cmap_name(name: &str) -> &str {
    match name {
        "H" | "V" => "H",
        _ => name
            .strip_suffix("-H")
            .or_else(|| name.strip_suffix("-V"))
            .unwrap_or(name),
    }
}

/// Unicode CMaps carry code points directly instead of CIDs.
fn is_unicode_cmap(base: &str) -> bool {
    base.starts_with("Uni") && (base.contains("UCS2") || base.contains("UTF16"))
}

/// Map a `/CIDSystemInfo` ordering to the generated table's collection name.
fn collection_of(ordering: &str) -> Option<&'static str> {
    // Orderings carry a supplement suffix in some documents (e.g. "Korea1-2").
    match ordering {
        o if o.starts_with("Korea1") => Some("KOREA1"),
        o if o.starts_with("Japan1") => Some("JAPAN1"),
        o if o.starts_with("GB1") => Some("GB1"),
        o if o.starts_with("CNS1") => Some("CNS1"),
        _ => None,
    }
}

fn find_table(ordering: &str, base: &str) -> Option<&'static PredefinedCmap> {
    let collection = collection_of(ordering)?;
    PREDEFINED_CMAPS
        .iter()
        .find(|cmap| cmap.collection == collection && cmap.column == base)
}

fn decode_utf16be(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok().filter(|s| !s.is_empty())
}

/// Walk the code stream, resolving each code to a CID and then to a character.
///
/// Codes are one or two bytes; see [`code_width`] for how the boundary is found.
/// Unmappable codes are skipped rather than emitted as replacement characters — a
/// partially mapped string is still useful, but garbage is not.
fn decode_with_table(
    bytes: &[u8],
    cmap: &PredefinedCmap,
    registry: &str,
    ordering: &str,
) -> Option<String> {
    let mut result = String::new();
    let mut any_mapped = false;
    let mut i = 0;

    while i < bytes.len() {
        let width = code_width(cmap, &bytes[i..]);
        let code = if width == 2 {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        } else {
            bytes[i] as u16
        };
        i += width;

        let mapped = cmap
            .codes
            .binary_search_by_key(&code, |&(c, _)| c)
            .ok()
            .and_then(|idx| lookup_cid(registry, ordering, cmap.codes[idx].1 as u32))
            .or_else(|| ascii_fallback(code, width));

        if let Some(ch) = mapped {
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

/// Number of bytes the next character code occupies.
///
/// A byte the table lists as a lead byte always starts a two-byte code, and a byte
/// below 0x80 is always a single-byte code. A high byte that is neither a lead byte
/// nor a valid single-byte code belongs to a region the CMap does not cover (e.g.
/// the Shift-JIS user-defined area): it is still a two-byte code, and consuming both
/// bytes is what keeps the rest of the string aligned — treating it as one byte would
/// turn every trail byte into a spurious character.
fn code_width(cmap: &PredefinedCmap, rest: &[u8]) -> usize {
    let byte = rest[0];
    if byte < 0x80 || rest.len() < 2 {
        return 1;
    }
    if cmap.lead_bytes.contains(&byte) || !contains_code(cmap, byte as u16) {
        return 2;
    }
    1
}

fn contains_code(cmap: &PredefinedCmap, code: u16) -> bool {
    cmap.codes.binary_search_by_key(&code, |&(c, _)| c).is_ok()
}

/// Resolve a single-byte code the character collection leaves unmapped.
///
/// The half-width Latin CIDs of the CJK collections (e.g. Adobe-Korea1 8094–8190)
/// have no entry in the CID→Unicode tables because their code *is* the character:
/// every encoding these CMaps describe keeps ASCII in the single-byte range.
fn ascii_fallback(code: u16, width: usize) -> Option<char> {
    match (width, code) {
        (1, 0x20..=0x7E) => Some(code as u8 as char),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_strips_writing_mode() {
        assert_eq!(base_cmap_name("KSC-EUC-H"), "KSC-EUC");
        assert_eq!(base_cmap_name("KSC-EUC-V"), "KSC-EUC");
        assert_eq!(base_cmap_name("UniJIS-UCS2-HW-V"), "UniJIS-UCS2-HW");
        assert_eq!(base_cmap_name("H"), "H");
        assert_eq!(base_cmap_name("V"), "H");
        assert_eq!(base_cmap_name("Identity-H"), "Identity");
    }

    #[test]
    fn decodes_euc_kr() {
        // C7D1 B1DB = "한글" in EUC-KR
        let decoded = decode("KSC-EUC-H", "Adobe", "Korea1", &[0xC7, 0xD1, 0xB1, 0xDB]);
        assert_eq!(decoded.as_deref(), Some("한글"));
    }

    #[test]
    fn decodes_euc_kr_mixed_with_ascii() {
        let decoded = decode("KSC-EUC-V", "Adobe", "Korea1-2", &[0x41, 0xC7, 0xD1, 0x42]);
        assert_eq!(decoded.as_deref(), Some("A한B"));
    }

    #[test]
    fn decodes_shift_jis() {
        // 82A0 82A2 = "あい" in Shift-JIS
        let decoded = decode("90ms-RKSJ-H", "Adobe", "Japan1", &[0x82, 0xA0, 0x82, 0xA2]);
        assert_eq!(decoded.as_deref(), Some("あい"));
    }

    #[test]
    fn decodes_gbk() {
        // D6D0 CEC4 = "中文" in GBK
        let decoded = decode("GBK-EUC-H", "Adobe", "GB1", &[0xD6, 0xD0, 0xCE, 0xC4]);
        assert_eq!(decoded.as_deref(), Some("中文"));
    }

    #[test]
    fn decodes_big5() {
        // A4A4 A4E5 = "中文" in Big5
        let decoded = decode("ETen-B5-H", "Adobe", "CNS1", &[0xA4, 0xA4, 0xA4, 0xE5]);
        assert_eq!(decoded.as_deref(), Some("中文"));
    }

    #[test]
    fn decodes_unicode_cmap_without_table() {
        let decoded = decode("UniKS-UCS2-H", "Adobe", "Korea1", &[0xD5, 0x5C, 0xAE, 0x00]);
        assert_eq!(decoded.as_deref(), Some("한글"));
    }

    /// A byte in a region the CMap leaves unmapped (here the Shift-JIS user-defined
    /// area) still starts a two-byte code — consuming only one byte would make every
    /// trail byte decode as a stray ASCII character.
    #[test]
    fn unmapped_lead_byte_does_not_desync() {
        let decoded = decode(
            "90ms-RKSJ-H",
            "Adobe",
            "Japan1",
            &[0xF0, 0x40, 0x82, 0xA0, 0xF0, 0x41],
        );
        assert_eq!(decoded.as_deref(), Some("あ"));
    }

    #[test]
    fn unsupported_cmap_yields_none() {
        assert_eq!(
            decode("KSC-Johab-H", "Adobe", "Korea1", &[0xC7, 0xD1]),
            None
        );
        assert_eq!(decode("KSC-EUC-H", "Adobe", "Unknown", &[0xC7, 0xD1]), None);
        assert_eq!(decode("KSC-EUC-H", "Adobe", "Korea1", &[]), None);
    }
}
