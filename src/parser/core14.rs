//! What a reader must know about the 14 standard Type 1 fonts on its own.
//!
//! A simple font normally carries `/FirstChar` and `/Widths`. The standard 14
//! fonts are the exception: a PDF before 1.5 may name one by `/BaseFont` alone and
//! leave its metrics to the reader (ISO 32000-1 §9.6.2.2). Without them a run has
//! no measured extent and falls back to a per-character estimate, which is exactly
//! the guess [`super::glyph_metrics`] exists to replace.
//!
//! The same fonts also need no `/Encoding`: a font without one uses its built-in
//! encoding (§9.6.6) — StandardEncoding for the Latin faces, and an encoding of
//! their own for Symbol and ZapfDingbats, whose codes would otherwise read as
//! unrelated Latin letters (a ZapfDingbats check mark is code `4`).
//!
//! Both are generated into [`super::core14_data`].

use std::collections::HashMap;
use std::sync::OnceLock;

use super::core14_data as data;
use super::encoding::{build_encoding_map, BaseEncoding};

/// One of the standard 14 fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StandardFont {
    /// Courier, Helvetica or Times — glyphs addressed by Unicode code point, so any
    /// encoding the font dictionary selects can be applied first.
    Latin(&'static [u16; data::LATIN_CHARS.len()]),
    /// Symbol — glyphs addressed by code in its own built-in encoding.
    Symbol,
    /// ZapfDingbats — glyphs addressed by code in its own built-in encoding.
    ZapfDingbats,
}

impl StandardFont {
    /// Recognise a standard font from its `/BaseFont` name.
    ///
    /// Accepts the 14 canonical names and the alternate names readers have long
    /// substituted for them (`Arial` → Helvetica, `TimesNewRoman` → Times,
    /// `CourierNew` → Courier, with `,Bold`/`-BoldMT`-style style suffixes).
    /// A subset tag (`ABCDEF+`) is ignored. Anything else is `None`.
    pub(crate) fn from_base_font(base_font: &[u8]) -> Option<Self> {
        let name = std::str::from_utf8(base_font).ok()?;
        let canonical = match name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => {
                rest
            }
            _ => name,
        };
        match canonical {
            "Symbol" => return Some(Self::Symbol),
            "ZapfDingbats" => return Some(Self::ZapfDingbats),
            _ => {}
        }

        let (family, style) = split_family(canonical)?;
        let lower = style.to_ascii_lowercase();
        let bold = lower.contains("bold");
        let slanted = lower.contains("italic") || lower.contains("oblique");
        let widths = match (family, bold, slanted) {
            (Family::Courier, false, false) => &data::COURIER,
            (Family::Courier, true, false) => &data::COURIER_BOLD,
            (Family::Courier, false, true) => &data::COURIER_OBLIQUE,
            (Family::Courier, true, true) => &data::COURIER_BOLD_OBLIQUE,
            (Family::Helvetica, false, false) => &data::HELVETICA,
            (Family::Helvetica, true, false) => &data::HELVETICA_BOLD,
            (Family::Helvetica, false, true) => &data::HELVETICA_OBLIQUE,
            (Family::Helvetica, true, true) => &data::HELVETICA_BOLD_OBLIQUE,
            (Family::Times, false, false) => &data::TIMES_ROMAN,
            (Family::Times, true, false) => &data::TIMES_BOLD,
            (Family::Times, false, true) => &data::TIMES_ITALIC,
            (Family::Times, true, true) => &data::TIMES_BOLD_ITALIC,
        };
        Some(Self::Latin(widths))
    }

    /// Advance widths for every byte code, in thousandths of text space.
    ///
    /// `encoding` is the font's code → character map (its `/Encoding`, or
    /// [`Self::builtin_encoding`] when it has none). It is ignored for Symbol and
    /// ZapfDingbats, whose codes index their own built-in encoding. A code with no
    /// glyph gets 0 — the text decoder drops the same code, so the run's extent
    /// and its text stay consistent.
    pub(crate) fn widths_by_code(&self, encoding: &HashMap<u8, char>) -> Vec<f32> {
        let by_code = |widths: &[u16; 256]| widths.iter().map(|&w| f32::from(w)).collect();
        match self {
            Self::Latin(widths) => (0u8..=255)
                .map(|code| {
                    encoding
                        .get(&code)
                        .and_then(|ch| data::LATIN_CHARS.binary_search(ch).ok())
                        .map_or(0.0, |i| f32::from(widths[i]))
                })
                .collect(),
            Self::Symbol => by_code(&data::SYMBOL),
            Self::ZapfDingbats => by_code(&data::ZAPF_DINGBATS),
        }
    }

    /// The code → character map the font uses when its dictionary has no
    /// `/Encoding` (ISO 32000-1 §9.6.6): StandardEncoding for the Latin faces, and
    /// each symbolic font's own encoding. Built once, shared by every font.
    pub(crate) fn builtin_encoding(&self) -> &'static HashMap<u8, char> {
        static STANDARD: OnceLock<HashMap<u8, char>> = OnceLock::new();
        static SYMBOL: OnceLock<HashMap<u8, char>> = OnceLock::new();
        static ZAPF_DINGBATS: OnceLock<HashMap<u8, char>> = OnceLock::new();
        match self {
            Self::Latin(_) => {
                STANDARD.get_or_init(|| build_encoding_map(Some(BaseEncoding::Standard), &[]))
            }
            Self::Symbol => SYMBOL.get_or_init(|| from_unicode_table(&data::SYMBOL_UNICODE)),
            Self::ZapfDingbats => {
                ZAPF_DINGBATS.get_or_init(|| from_unicode_table(&data::ZAPF_DINGBATS_UNICODE))
            }
        }
    }
}

/// A generated code → scalar-value table as a map, 0 meaning "no glyph".
fn from_unicode_table(table: &[u32; 256]) -> HashMap<u8, char> {
    (0u8..=255)
        .filter_map(|code| match table[usize::from(code)] {
            0 => None,
            cp => Some((code, char::from_u32(cp)?)),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Courier,
    Helvetica,
    Times,
}

/// Split a standard-font name into its family and whatever style text follows.
fn split_family(name: &str) -> Option<(Family, &str)> {
    // Longest prefixes first: `CourierNew` before `Courier`, `TimesNewRoman` before
    // `Times`. The style part keeps its separator (`-Bold`, `,Italic`, `PS-BoldMT`).
    const PREFIXES: [(&str, Family); 6] = [
        ("CourierNew", Family::Courier),
        ("Courier", Family::Courier),
        ("Helvetica", Family::Helvetica),
        ("Arial", Family::Helvetica),
        ("TimesNewRoman", Family::Times),
        ("Times", Family::Times),
    ];
    PREFIXES.iter().find_map(|(prefix, family)| {
        let style = name.strip_prefix(prefix)?;
        // `Times-Roman` is the upright face; any other text after the family must
        // start a style suffix, so `Arialic` or `HelveticaNeue` are not taken.
        let ok = style.is_empty()
            || style.starts_with(['-', ','])
            || style.starts_with("MT")
            || style.starts_with("PS");
        ok.then_some((*family, style))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latin(name: &str) -> &'static [u16; data::LATIN_CHARS.len()] {
        match StandardFont::from_base_font(name.as_bytes()) {
            Some(StandardFont::Latin(w)) => w,
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn recognises_the_fourteen_canonical_names() {
        for name in [
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            "Symbol",
            "ZapfDingbats",
        ] {
            assert!(
                StandardFont::from_base_font(name.as_bytes()).is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn maps_each_style_to_its_own_face() {
        assert!(std::ptr::eq(latin("Times-Roman"), &data::TIMES_ROMAN));
        assert!(std::ptr::eq(
            latin("Times-BoldItalic"),
            &data::TIMES_BOLD_ITALIC
        ));
        assert!(std::ptr::eq(
            latin("Helvetica-Oblique"),
            &data::HELVETICA_OBLIQUE
        ));
        assert!(std::ptr::eq(latin("Courier-Bold"), &data::COURIER_BOLD));
    }

    #[test]
    fn accepts_alternate_names_and_subset_tags() {
        assert!(std::ptr::eq(latin("Arial"), &data::HELVETICA));
        assert!(std::ptr::eq(latin("ArialMT"), &data::HELVETICA));
        assert!(std::ptr::eq(latin("Arial,Bold"), &data::HELVETICA_BOLD));
        assert!(std::ptr::eq(
            latin("Arial-BoldItalicMT"),
            &data::HELVETICA_BOLD_OBLIQUE
        ));
        assert!(std::ptr::eq(latin("TimesNewRoman"), &data::TIMES_ROMAN));
        assert!(std::ptr::eq(
            latin("TimesNewRomanPS-BoldMT"),
            &data::TIMES_BOLD
        ));
        assert!(std::ptr::eq(
            latin("CourierNew,Italic"),
            &data::COURIER_OBLIQUE
        ));
        assert!(std::ptr::eq(latin("ABCDEF+Helvetica"), &data::HELVETICA));
    }

    #[test]
    fn rejects_other_fonts_that_merely_share_a_prefix() {
        for name in [
            "HelveticaNeue",
            "Arialic",
            "TimesTen-Roman",
            "Calibri",
            "abcdef+Helvetica",
        ] {
            assert_eq!(
                StandardFont::from_base_font(name.as_bytes()),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn latin_widths_follow_the_selected_encoding() {
        // Helvetica AFM: space 278, `m` 833, `i` 222; WinAnsi 0x80 is the Euro (556).
        let font = StandardFont::from_base_font(b"Helvetica").unwrap();
        let widths = font.widths_by_code(&build_encoding_map(Some(BaseEncoding::WinAnsi), &[]));
        assert_eq!(widths[b' ' as usize], 278.0);
        assert_eq!(widths[b'm' as usize], 833.0);
        assert_eq!(widths[b'i' as usize], 222.0);
        assert_eq!(widths[0x80], 556.0);
        // A /Differences entry moves a glyph to another code.
        let custom = build_encoding_map(None, &[(0x01, "m".to_string())]);
        assert_eq!(font.widths_by_code(&custom)[0x01], 833.0);
    }

    #[test]
    fn courier_is_monospaced_and_unmapped_codes_are_zero() {
        let font = StandardFont::from_base_font(b"Courier").unwrap();
        let widths = font.widths_by_code(&build_encoding_map(Some(BaseEncoding::WinAnsi), &[]));
        assert!(widths[0x20..0x7F].iter().all(|&w| w == 600.0));
        assert_eq!(widths[0x00], 0.0);
    }

    #[test]
    fn built_in_encodings_name_the_glyph_the_font_draws() {
        let symbol = StandardFont::from_base_font(b"Symbol")
            .unwrap()
            .builtin_encoding();
        assert_eq!(symbol.get(&b'a'), Some(&'\u{03B1}')); // alpha
        assert_eq!(symbol.get(&b'D'), Some(&'\u{0394}')); // Delta
        let dingbats = StandardFont::from_base_font(b"ZapfDingbats")
            .unwrap()
            .builtin_encoding();
        assert_eq!(dingbats.get(&b'4'), Some(&'\u{2714}')); // heavy check mark
        assert_eq!(dingbats.get(&b'l'), Some(&'\u{25CF}')); // black circle
        assert_eq!(dingbats.get(&0x00), None);
        // The Latin faces use StandardEncoding: 0x27 is a right single quote there.
        let helvetica = StandardFont::from_base_font(b"Helvetica")
            .unwrap()
            .builtin_encoding();
        assert_eq!(helvetica.get(&0x27), Some(&'\u{2019}'));
    }

    #[test]
    fn symbolic_fonts_use_their_built_in_codes() {
        // Symbol AFM: code 0x61 is `alpha` (631); ZapfDingbats 0x21 is `a1` (974).
        let empty = HashMap::new();
        let symbol = StandardFont::from_base_font(b"Symbol").unwrap();
        assert_eq!(symbol.widths_by_code(&empty)[0x61], 631.0);
        let dingbats = StandardFont::from_base_font(b"ZapfDingbats").unwrap();
        assert_eq!(dingbats.widths_by_code(&empty)[0x21], 974.0);
    }
}
