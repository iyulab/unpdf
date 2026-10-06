//! Bare CFF font programs (`/FontFile3` `/Type1C`): what text extraction needs from them —
//! the program's own encoding, as characters (Adobe Technical Note #5176).
//!
//! A CFF font maps a code to a glyph through its `Encoding`, and a glyph to a name through
//! its `charset` (a string ID per glyph, IDs below 391 being the standard strings). The name
//! gives the character. Only the structures that lead there are read: the header, the
//! Name, Top DICT and String INDEXes, the Top DICT's `charset`, `Encoding` and
//! `CharStrings` operators, and the charset and encoding tables. A CID-keyed font (`ROS` in
//! its Top DICT) maps codes through a CMap instead and is not read here.

use std::collections::HashMap;

use super::cff_strings::STANDARD_STRINGS;
use super::encoding::{glyph_name_to_unicode, BaseEncoding};

/// What each code is in a bare CFF program's built-in encoding — the encoding a font with no
/// `/Encoding` uses, and the base its `/Differences` apply to when it names none (ISO
/// 32000-1 §9.6.6.1). `None` when `data` is not a CFF program this reads (CID-keyed, the
/// Expert encoding or charsets, or damaged).
pub(crate) fn builtin_encoding_chars(data: &[u8]) -> Option<HashMap<u8, char>> {
    let header_size = usize::from(*data.get(2)?);
    let (_names, at) = index(data, header_size)?;
    let (top_dicts, at) = index(data, at)?;
    let (strings, _) = index(data, at)?;
    let top = Dict::parse(top_dicts.first()?)?;
    if top.ros {
        return None;
    }
    let (charstrings, _) = index(data, top.charstrings?)?;
    let glyphs = charstrings.len();

    // A string ID: the standard strings, then the font's own.
    let string = |sid: u16| -> Option<&str> {
        let sid = usize::from(sid);
        match sid.checked_sub(STANDARD_STRINGS.len()) {
            None => STANDARD_STRINGS.get(sid).copied(),
            Some(custom) => std::str::from_utf8(strings.get(custom)?).ok(),
        }
    };
    let codes = match top.encoding {
        0 => {
            return Some(
                (0..=255u8)
                    .filter_map(|code| Some((code, BaseEncoding::Standard.decode_char(code)?)))
                    .collect(),
            )
        }
        1 => return None,
        offset => encoding_codes(data, offset)?,
    };
    Some(
        codes
            .into_iter()
            .filter_map(|(code, glyph)| {
                let sid = match glyph {
                    Glyph::Index(gid) => charset_sid(data, top.charset, glyphs, gid)?,
                    Glyph::String(sid) => sid,
                };
                Some((code, glyph_name_to_unicode(string(sid)?)?))
            })
            .collect(),
    )
}

/// What an encoding entry points at: a glyph by index, or (a supplement) by its name's
/// string ID.
enum Glyph {
    Index(usize),
    String(u16),
}

/// A custom encoding's codes (format 0 or 1, with supplements).
fn encoding_codes(data: &[u8], offset: usize) -> Option<Vec<(u8, Glyph)>> {
    let format = *data.get(offset)?;
    let mut codes = Vec::new();
    let mut at = offset + 1;
    match format & 0x7F {
        0 => {
            let n = usize::from(*data.get(at)?);
            for (i, &code) in data.get(at + 1..at + 1 + n)?.iter().enumerate() {
                codes.push((code, Glyph::Index(i + 1)));
            }
            at += 1 + n;
        }
        1 => {
            let ranges = usize::from(*data.get(at)?);
            let mut gid = 1;
            for r in 0..ranges {
                let first = *data.get(at + 1 + 2 * r)?;
                let left = *data.get(at + 2 + 2 * r)?;
                for code in first..=first.saturating_add(left) {
                    codes.push((code, Glyph::Index(gid)));
                    gid += 1;
                }
            }
            at += 1 + 2 * ranges;
        }
        _ => return None,
    }
    if format & 0x80 != 0 {
        let n = usize::from(*data.get(at)?);
        for s in 0..n {
            let entry = data.get(at + 1 + 3 * s..at + 4 + 3 * s)?;
            codes.push((
                entry[0],
                Glyph::String(u16::from_be_bytes([entry[1], entry[2]])),
            ));
        }
    }
    Some(codes)
}

/// The string ID of glyph `gid`'s name. Charset 0 is ISOAdobe, where glyph `n` is string
/// `n`; the Expert charsets (1, 2) are not read.
fn charset_sid(data: &[u8], charset: usize, glyphs: usize, gid: usize) -> Option<u16> {
    if gid == 0 {
        return Some(0);
    }
    match charset {
        0 => u16::try_from(gid).ok().filter(|&sid| sid < 229),
        1 | 2 => None,
        offset => {
            let format = *data.get(offset)?;
            let mut at = offset + 1;
            match format {
                0 => {
                    let b = data.get(at + 2 * (gid - 1)..at + 2 * gid)?;
                    Some(u16::from_be_bytes([b[0], b[1]]))
                }
                1 | 2 => {
                    let left_size = if format == 1 { 1 } else { 2 };
                    let mut current = 1;
                    while current < glyphs {
                        let first = u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]);
                        let left = if left_size == 1 {
                            usize::from(*data.get(at + 2)?)
                        } else {
                            usize::from(u16::from_be_bytes([
                                *data.get(at + 2)?,
                                *data.get(at + 3)?,
                            ]))
                        };
                        if gid < current + left + 1 {
                            return first.checked_add(u16::try_from(gid - current).ok()?);
                        }
                        current += left + 1;
                        at += 2 + left_size;
                    }
                    None
                }
                _ => None,
            }
        }
    }
}

/// The Top DICT entries this reads.
struct Dict {
    charset: usize,
    encoding: usize,
    charstrings: Option<usize>,
    ros: bool,
}

impl Dict {
    fn parse(dict: &[u8]) -> Option<Self> {
        let mut out = Dict {
            charset: 0,
            encoding: 0,
            charstrings: None,
            ros: false,
        };
        let mut operands: Vec<i64> = Vec::new();
        let mut i = 0;
        while i < dict.len() {
            let b0 = dict[i];
            i += 1;
            match b0 {
                32..=246 => operands.push(i64::from(b0) - 139),
                247..=250 => {
                    operands.push((i64::from(b0) - 247) * 256 + i64::from(*dict.get(i)?) + 108);
                    i += 1;
                }
                251..=254 => {
                    operands.push(-(i64::from(b0) - 251) * 256 - i64::from(*dict.get(i)?) - 108);
                    i += 1;
                }
                28 => {
                    let b = dict.get(i..i + 2)?;
                    operands.push(i64::from(i16::from_be_bytes([b[0], b[1]])));
                    i += 2;
                }
                29 => {
                    let b = dict.get(i..i + 4)?;
                    operands.push(i64::from(i32::from_be_bytes([b[0], b[1], b[2], b[3]])));
                    i += 4;
                }
                // A real number: nibbles until one is 0xF. Its value is not needed here.
                30 => {
                    while i < dict.len() {
                        let b = dict[i];
                        i += 1;
                        if b & 0x0F == 0x0F || b >> 4 == 0x0F {
                            break;
                        }
                    }
                    operands.push(0);
                }
                12 => {
                    let op = *dict.get(i)?;
                    i += 1;
                    if op == 30 {
                        out.ros = true;
                    }
                    operands.clear();
                }
                op => {
                    let last = operands
                        .last()
                        .copied()
                        .and_then(|v| usize::try_from(v).ok());
                    match op {
                        15 => out.charset = last?,
                        16 => out.encoding = last?,
                        17 => out.charstrings = last,
                        _ => {}
                    }
                    operands.clear();
                }
            }
        }
        Some(out)
    }
}

/// The items of the INDEX at `at`, and where the next structure starts.
fn index(data: &[u8], at: usize) -> Option<(Vec<&[u8]>, usize)> {
    let count = usize::from(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]));
    if count == 0 {
        return Some((Vec::new(), at + 2));
    }
    let off_size = usize::from(*data.get(at + 2)?);
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let offsets_at = at + 3;
    let offset = |k: usize| -> Option<usize> {
        let bytes = data.get(offsets_at + k * off_size..offsets_at + (k + 1) * off_size)?;
        Some(
            bytes
                .iter()
                .fold(0usize, |acc, &b| (acc << 8) | usize::from(b)),
        )
    };
    // Offsets count from the byte before the data.
    let base = offsets_at + (count + 1) * off_size - 1;
    let mut items = Vec::with_capacity(count);
    for k in 0..count {
        let (start, end) = (offset(k)?, offset(k + 1)?);
        items.push(data.get(base + start..base + end)?);
    }
    Some((items, base + offset(count)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An INDEX of `items` with 1-byte offsets.
    fn index_bytes(items: &[&[u8]]) -> Vec<u8> {
        let mut out = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return out;
        }
        out.push(1);
        let mut offset = 1u8;
        out.push(offset);
        for item in items {
            offset += item.len() as u8;
            out.push(offset);
        }
        for item in items {
            out.extend_from_slice(item);
        }
        out
    }

    /// A DICT integer, in the 2-byte form (operator 28).
    fn int(v: i16) -> Vec<u8> {
        let mut out = vec![28];
        out.extend(v.to_be_bytes());
        out
    }

    /// A CFF program with glyphs named `names` (custom strings after the standard ones) and
    /// the given encoding table bytes (`None`: the standard encoding).
    fn program(names: &[&str], encoding: Option<Vec<u8>>, extra_top: &[u8]) -> Vec<u8> {
        let header = [1u8, 0, 4, 1];
        let name_index = index_bytes(&[b"Test"]);
        let custom: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !STANDARD_STRINGS.contains(n))
            .collect();
        let string_items: Vec<&[u8]> = custom.iter().map(|s| s.as_bytes()).collect();
        let string_index = index_bytes(&string_items);
        let gsubr = index_bytes(&[]);
        let sid = |n: &str| -> u16 {
            STANDARD_STRINGS
                .iter()
                .position(|s| *s == n)
                .map(|p| p as u16)
                .unwrap_or_else(|| 391 + custom.iter().position(|s| *s == n).unwrap() as u16)
        };
        // Charset format 0 (glyphs after .notdef), encoding, charstrings: each a trivial
        // `endchar`.
        let mut charset = vec![0u8];
        for n in names {
            charset.extend(sid(n).to_be_bytes());
        }
        let charstring_items: Vec<&[u8]> = (0..=names.len()).map(|_| &[14u8][..]).collect();
        let charstrings = index_bytes(&charstring_items);

        // The Top DICT's size is fixed (2-byte integers), so offsets can be computed first:
        // three operands of 3 bytes, three operators, and `extra_top`.
        let top_len = 3 * 3 + 3 + extra_top.len();
        let top_index_len = 2 + 1 + 2 + top_len;
        let start =
            header.len() + name_index.len() + top_index_len + string_index.len() + gsubr.len();
        let charset_at = start;
        let encoding_at = charset_at + charset.len();
        let charstrings_at = encoding_at + encoding.as_ref().map_or(0, Vec::len);
        let mut top = Vec::new();
        top.extend(int(charset_at as i16));
        top.push(15);
        top.extend(int(if encoding.is_some() {
            encoding_at as i16
        } else {
            0
        }));
        top.push(16);
        top.extend(int(charstrings_at as i16));
        top.push(17);
        top.extend_from_slice(extra_top);
        let top_index = index_bytes(&[&top]);
        assert_eq!(top_index.len(), top_index_len);

        let mut out = header.to_vec();
        out.extend(name_index);
        out.extend(top_index);
        out.extend(string_index);
        out.extend(gsubr);
        out.extend(charset);
        out.extend(encoding.unwrap_or_default());
        out.extend(charstrings);
        out
    }

    #[test]
    fn a_custom_encoding_maps_codes_to_named_glyphs() {
        // Format 0: glyph 1 at code 11, glyph 2 at code 92, glyph 3 at 'c'.
        let enc = vec![0, 3, 11, 92, b'c'];
        let data = program(&["ff", "quotedblleft", "c"], Some(enc), &[]);
        let map = builtin_encoding_chars(&data).expect("reads");
        assert_eq!(map.get(&11), Some(&'\u{FB00}'));
        assert_eq!(map.get(&92), Some(&'\u{201C}'));
        assert_eq!(map.get(&b'c'), Some(&'c'));
        assert_eq!(map.get(&b'd'), None);
    }

    #[test]
    fn ranges_and_supplements_are_read() {
        // Format 1 with supplements: codes 65..=66 for glyphs 1-2, and code 200 also names
        // `B` (standard string 35).
        let enc = vec![0x81, 1, 65, 1, 1, 200, 0, 35];
        let data = program(&["A", "B"], Some(enc), &[]);
        let map = builtin_encoding_chars(&data).expect("reads");
        assert_eq!(map.get(&65), Some(&'A'));
        assert_eq!(map.get(&66), Some(&'B'));
        assert_eq!(map.get(&200), Some(&'B'));
    }

    #[test]
    fn the_standard_encoding_is_standard_encoding() {
        let data = program(&["A"], None, &[]);
        let map = builtin_encoding_chars(&data).expect("reads");
        assert_eq!(map.get(&0x27), Some(&'\u{2019}'));
    }

    #[test]
    fn a_cid_keyed_font_is_not_read_here() {
        // ROS (12 30) in the Top DICT.
        let data = program(&["A"], Some(vec![0, 1, 65]), &[139, 139, 139, 12, 30]);
        assert!(builtin_encoding_chars(&data).is_none());
    }

    #[test]
    fn garbage_is_not_a_cff_program() {
        assert!(builtin_encoding_chars(b"").is_none());
        assert!(builtin_encoding_chars(b"%!PS-AdobeFont").is_none());
    }
}
