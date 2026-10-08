//! Predefined CJK CMap decoding for Type0 fonts.
//!
//! A composite font may reference one of Adobe's predefined CMaps by name
//! (`/Encoding /KSC-EUC-H`) instead of embedding a ToUnicode CMap. Those names
//! describe a legacy character encoding (EUC-KR, Shift-JIS, GBK, Big5) or a
//! Unicode encoding, plus a writing mode:
//!
//! ```text
//! KSC-EUC-H       →  KS X 1001 charset, EUC-KR encoding, horizontal
//! UniKS-UCS2-V    →  UCS-2 (UTF-16BE), vertical
//! UniJIS-UTF16-H  →  UTF-16BE, horizontal
//! UniGB-UTF8-H    →  UTF-8, horizontal
//! UniCNS-UTF32-V  →  UTF-32BE, vertical
//! ```
//!
//! Legacy CMaps map character codes to CIDs, which the character collection's
//! CID→Unicode table then resolves. Unicode CMaps encode the code points directly,
//! so no table is needed to read them. Writing mode does not affect the mapping, only
//! glyph selection, so `-H` and `-V` share a table.
//!
//! [`split`] cuts a string into the codes of one such CMap, each with its CID (or, for a
//! Unicode CMap, its character). [`super::cmap::CMap`] builds text and widths on it.
//!
//! CMaps outside the shipped set split to `None`, which the caller treats the
//! same as any other unusable CMap — no text rather than mojibake.

use super::cmap_table::{cid_of_char, PredefinedCmap, PREDEFINED_CMAPS};

/// Whether [`split`] can resolve CIDs under `encoding_name` for a CIDFont of `ordering`.
pub(crate) fn resolves_cids(encoding_name: &str, ordering: &str) -> bool {
    let base = base_cmap_name(encoding_name);
    match unicode_form(base) {
        Some(_) => cid_of_char(ordering, 'A').is_some(),
        None => find_table(ordering, base).is_some(),
    }
}

/// One character code of a string under a predefined CMap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PredefinedCode {
    /// The code as a big-endian number.
    pub value: u32,
    /// Bytes the code occupies.
    pub len: usize,
    /// The CID the CMap gives the code; 0 (`.notdef`) when it does not map it.
    pub cid: u32,
    /// The character a Unicode CMap carries in the code itself.
    pub scalar: Option<char>,
}

/// Split `bytes` into the codes of a predefined CMap, the way the CMap's own code space
/// does. `None` when the CMap or the collection is not supported.
pub(crate) fn split(
    encoding_name: &str,
    ordering: &str,
    bytes: &[u8],
) -> Option<Vec<PredefinedCode>> {
    let base = base_cmap_name(encoding_name);
    if let Some(form) = unicode_form(base) {
        let mut at = 0;
        return Some(
            unicode_codes(form, bytes)
                .into_iter()
                .map(|code| {
                    let value = bytes[at..at + code.len]
                        .iter()
                        .fold(0u32, |acc, &b| acc << 8 | u32::from(b));
                    at += code.len;
                    PredefinedCode {
                        value,
                        len: code.len,
                        cid: code
                            .scalar
                            .and_then(|ch| cid_of_char(ordering, ch))
                            .unwrap_or(0),
                        scalar: code.scalar,
                    }
                })
                .collect(),
        );
    }

    let cmap = find_table(ordering, base)?;
    let mut codes = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let (code, width) = next_code(cmap, &bytes[i..]);
        i += width;
        let cid = cmap
            .codes
            .binary_search_by_key(&code, |&(c, _)| c)
            .map_or(0, |idx| u32::from(cmap.codes[idx].1));
        codes.push(PredefinedCode {
            value: u32::from(code),
            len: width,
            cid,
            scalar: None,
        });
    }
    Some(codes)
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

/// How a Unicode CMap writes its code points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnicodeForm {
    /// `UCS2` and `UTF16`: big-endian 16-bit units. UCS-2 is the surrogate-free subset.
    Utf16,
    Utf8,
    /// Big-endian 32-bit units.
    Utf32,
}

/// The encoding form of a Unicode CMap — `UniKS-UCS2`, `UniJIS2004-UTF16`,
/// `UniJISX0213-UTF32`, `UniGB-UTF8` and the rest of Adobe's `Uni*` family — or `None`
/// for a legacy CMap.
fn unicode_form(base: &str) -> Option<UnicodeForm> {
    if !base.starts_with("Uni") {
        return None;
    }
    if base.contains("UCS2") || base.contains("UTF16") {
        Some(UnicodeForm::Utf16)
    } else if base.contains("UTF8") {
        Some(UnicodeForm::Utf8)
    } else if base.contains("UTF32") {
        Some(UnicodeForm::Utf32)
    } else {
        None
    }
}

/// One code of a Unicode CMap string.
struct UnicodeCode {
    /// The character, or `None` for a malformed code: a lone surrogate, an invalid
    /// UTF-8 sequence, a value beyond U+10FFFF, a truncated unit.
    scalar: Option<char>,
    /// Bytes the code occupies.
    len: usize,
}

/// Split `bytes` into the codes of a Unicode CMap. A malformed code costs only itself —
/// the next code starts right after it, so one bad unit cannot discard its neighbours.
fn unicode_codes(form: UnicodeForm, bytes: &[u8]) -> Vec<UnicodeCode> {
    let mut codes = Vec::new();
    let rest = match form {
        UnicodeForm::Utf16 => {
            let (units, rest) = bytes.as_chunks::<2>();
            codes.extend(
                char::decode_utf16(units.iter().copied().map(u16::from_be_bytes)).map(|unit| {
                    UnicodeCode {
                        len: unit.as_ref().map_or(2, |ch| ch.len_utf16() * 2),
                        scalar: unit.ok(),
                    }
                }),
            );
            rest
        }
        UnicodeForm::Utf8 => {
            for chunk in bytes.utf8_chunks() {
                codes.extend(chunk.valid().chars().map(|ch| UnicodeCode {
                    scalar: Some(ch),
                    len: ch.len_utf8(),
                }));
                if !chunk.invalid().is_empty() {
                    codes.push(UnicodeCode {
                        scalar: None,
                        len: chunk.invalid().len(),
                    });
                }
            }
            &[]
        }
        UnicodeForm::Utf32 => {
            let (units, rest) = bytes.as_chunks::<4>();
            codes.extend(units.iter().map(|&unit| UnicodeCode {
                scalar: char::from_u32(u32::from_be_bytes(unit)),
                len: 4,
            }));
            rest
        }
    };
    if !rest.is_empty() {
        codes.push(UnicodeCode {
            scalar: None,
            len: rest.len(),
        });
    }
    codes
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

/// The next character code in `rest` and the number of bytes it occupies.
fn next_code(cmap: &PredefinedCmap, rest: &[u8]) -> (u16, usize) {
    match code_width(cmap, rest) {
        2 => (u16::from_be_bytes([rest[0], rest[1]]), 2),
        _ => (u16::from(rest[0]), 1),
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
pub(crate) fn ascii_fallback(code: u16, width: usize) -> Option<char> {
    match (width, code) {
        (1, 0x20..=0x7E) => Some(code as u8 as char),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::cmap::{CMap, Code};

    fn decode(name: &str, registry: &str, ordering: &str, bytes: &[u8]) -> Option<String> {
        CMap::predefined(name).decode(registry, ordering, bytes)
    }

    fn cids(name: &str, ordering: &str, bytes: &[u8]) -> Option<Vec<Code>> {
        CMap::predefined(name).codes(ordering, bytes)
    }

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

    #[test]
    fn every_unicode_encoding_form_decodes() {
        // 𠮷 (U+20BB7) lies outside the BMP: a surrogate pair in UTF-16.
        let utf16 = [0xD8, 0x42, 0xDF, 0xB7, 0x91, 0xCE];
        assert_eq!(
            decode("UniJIS-UTF16-V", "", "", &utf16).as_deref(),
            Some("𠮷野")
        );
        assert_eq!(
            decode("UniJIS-UTF8-H", "", "", "𠮷野".as_bytes()).as_deref(),
            Some("𠮷野")
        );
        let utf32 = [0x00, 0x02, 0x0B, 0xB7, 0x00, 0x00, 0x91, 0xCE];
        assert_eq!(
            decode("UniJISX0213-UTF32-H", "", "", &utf32).as_deref(),
            Some("𠮷野")
        );
    }

    /// One malformed code is skipped; the codes around it still read.
    #[test]
    fn a_malformed_unicode_code_costs_only_itself() {
        // A lone high surrogate between two characters.
        let utf16 = [0xD5, 0x5C, 0xD8, 0x00, 0xAE, 0x00];
        assert_eq!(
            decode("UniKS-UCS2-H", "", "", &utf16).as_deref(),
            Some("한글")
        );
        // A value beyond U+10FFFF, then a truncated unit.
        let utf32 = [0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0xD5, 0x5C, 0x00];
        assert_eq!(
            decode("UniKS-UTF32-H", "", "", &utf32).as_deref(),
            Some("한")
        );
        assert_eq!(
            decode("UniKS-UTF8-H", "", "", &[0xED, 0x95, 0x9C, 0xFF, 0x41]).as_deref(),
            Some("한A")
        );
    }

    #[test]
    fn unicode_codes_resolve_to_the_collections_cids() {
        // Adobe-Korea1: `A` is CID 34, 가 is CID 1086.
        let codes = cids("UniKS-UCS2-H", "Korea1", &[0x00, 0x41, 0xAC, 0x00]).unwrap();
        let got: Vec<u32> = codes.iter().map(|c| c.cid).collect();
        assert_eq!(got, vec![34, 1086]);
        // A malformed code still occupies a position — as `.notdef`.
        let codes = cids("UniKS-UCS2-H", "Korea1", &[0xD8, 0x00, 0x00, 0x41]).unwrap();
        let got: Vec<u32> = codes.iter().map(|c| c.cid).collect();
        assert_eq!(got, vec![0, 34]);
        // Under UTF-8 a space is the single-byte code 32, so word spacing applies.
        let codes = cids("UniKS-UTF8-H", "Korea1", b"A B").unwrap();
        assert_eq!(
            codes.iter().map(|c| c.is_word_space).collect::<Vec<_>>(),
            vec![false, true, false]
        );
        // Under UCS-2 it is two bytes, and word spacing does not apply.
        let codes = cids("UniKS-UCS2-H", "Korea1", &[0x00, 0x20]).unwrap();
        assert!(!codes[0].is_word_space);
    }

    #[test]
    fn cids_need_a_known_collection_and_cmap() {
        assert!(resolves_cids("UniKS-UCS2-H", "Korea1"));
        assert!(resolves_cids("KSC-EUC-H", "Korea1-2"));
        assert!(!resolves_cids("UniKS-UCS2-H", "Unknown"));
        assert!(!resolves_cids("KSC-Johab-H", "Korea1"));
        assert_eq!(cids("KSC-Johab-H", "Korea1", &[0x41]), None);
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
