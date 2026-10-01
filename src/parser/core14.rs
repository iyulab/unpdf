//! Metrics of the 14 standard Type 1 fonts, for fonts that declare no widths.
//!
//! A simple font normally carries `/FirstChar` and `/Widths`. The standard 14
//! fonts are the exception: a PDF before 1.5 may name one by `/BaseFont` alone and
//! leave its metrics to the reader (ISO 32000-1 §9.6.2.2). Without them a run has
//! no measured extent and falls back to a per-character estimate, which is exactly
//! the guess [`super::glyph_metrics`] exists to replace. The widths themselves are
//! generated from the Adobe AFM files into [`super::core14_data`].

use std::collections::HashMap;

use super::core14_data as data;

/// One of the standard 14 fonts, with the widths that go with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StandardFont {
    /// Courier, Helvetica or Times — glyphs addressed by Unicode code point, so any
    /// encoding the font dictionary selects can be applied first.
    Latin(&'static [u16; data::LATIN_CHARS.len()]),
    /// Symbol or ZapfDingbats — glyphs addressed by code in the font's own built-in
    /// encoding, which is what a font dictionary without `/Encoding` uses.
    Symbolic(&'static [u16; 256]),
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
        let name = match name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => {
                rest
            }
            _ => name,
        };
        let canonical = match name {
            "Symbol" => return Some(Self::Symbolic(&data::SYMBOL)),
            "ZapfDingbats" => return Some(Self::Symbolic(&data::ZAPF_DINGBATS)),
            other => other,
        };

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
    /// `encoding` is the font's code → character map (its `/Encoding`, or the
    /// standard encoding when it has none). It is ignored for Symbol and
    /// ZapfDingbats, whose codes index their own built-in encoding. A code with no
    /// glyph gets 0 — the text decoder drops the same code, so the run's extent
    /// and its text stay consistent.
    pub(crate) fn widths_by_code(&self, encoding: &HashMap<u8, char>) -> Vec<f32> {
        match self {
            Self::Latin(widths) => (0u8..=255)
                .map(|code| {
                    encoding
                        .get(&code)
                        .and_then(|ch| data::LATIN_CHARS.binary_search(ch).ok())
                        .map_or(0.0, |i| f32::from(widths[i]))
                })
                .collect(),
            Self::Symbolic(widths) => widths.iter().map(|&w| f32::from(w)).collect(),
        }
    }
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
    use crate::parser::encoding::{build_encoding_map, BaseEncoding};

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
    fn symbolic_fonts_use_their_built_in_codes() {
        // Symbol AFM: code 0x61 is `alpha` (631); ZapfDingbats 0x21 is `a1` (974).
        let empty = HashMap::new();
        let symbol = StandardFont::from_base_font(b"Symbol").unwrap();
        assert_eq!(symbol.widths_by_code(&empty)[0x61], 631.0);
        let dingbats = StandardFont::from_base_font(b"ZapfDingbats").unwrap();
        assert_eq!(dingbats.widths_by_code(&empty)[0x21], 974.0);
    }
}
