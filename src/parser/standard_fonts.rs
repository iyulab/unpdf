//! Stand-in outlines for fonts a PDF names but does not embed.
//!
//! A PDF may leave a font's program out and expect the reader to supply it: always for the
//! standard 14 fonts (ISO 32000-1 §9.6.2.2), and in practice for common system fonts such
//! as Arial or Times New Roman. Readers paint such text with a face of the same class —
//! sans, serif or fixed-pitch, upright or italic, regular or bold — and the PDF's own widths
//! keep the glyphs where the page put them. The faces here are the 14 fonts PDFium ships
//! for the purpose (BSD-3-Clause, `assets/standard-fonts/LICENSE`) — bare CFF programs,
//! metrically compatible with Helvetica, Times and Courier, plus Symbol and ZapfDingbats.

/// The standard face that stands in for a font named `base_font` whose descriptor's `/Flags`
/// are `flags` (`None` with no descriptor), or `None` when no face here is a fair stand-in —
/// a symbolic font of unknown design, whose codes select glyphs no Latin face has.
pub(crate) fn stand_in(
    base_font: &str,
    flags: Option<i64>,
    bold: bool,
    italic: bool,
) -> Option<&'static [u8]> {
    const FIXED_PITCH: i64 = 1;
    const SERIF: i64 = 1 << 1;
    const SYMBOLIC: i64 = 1 << 2;
    const NONSYMBOLIC: i64 = 1 << 5;
    const ITALIC: i64 = 1 << 6;
    const FORCE_BOLD: i64 = 1 << 18;

    // "ABCDEF+Arial,BoldItalic" → "arialbolditalic"
    let name: String = base_font
        .split_once('+')
        .filter(|(tag, _)| tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()))
        .map_or(base_font, |(_, rest)| rest)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    let flags = flags.unwrap_or(0);

    if has(&["symbol"]) {
        return Some(SYMBOL);
    }
    if has(&["dingbats", "zapf"]) {
        return Some(DINGBATS);
    }
    let fixed = flags & FIXED_PITCH != 0
        || has(&[
            "courier",
            "mono",
            "consol",
            "fixed",
            "typewriter",
            "lucidaconsole",
        ]);
    let serif = !fixed
        && !has(&[
            "sans",
            "gothic",
            "arial",
            "helvetica",
            "verdana",
            "tahoma",
            "calibri",
        ])
        && (flags & SERIF != 0
            || has(&[
                "times",
                "serif",
                "roman",
                "georgia",
                "garamond",
                "cambria",
                "palatino",
                "bookantiqua",
                "minion",
                "century",
                "baskerville",
                "caslon",
            ]));
    let known = fixed
        || serif
        || has(&[
            "sans",
            "arial",
            "helvetica",
            "verdana",
            "tahoma",
            "calibri",
            "segoe",
            "trebuchet",
            "frutiger",
            "univers",
            "myriad",
            "franklin",
        ]);
    if flags & SYMBOLIC != 0 && flags & NONSYMBOLIC == 0 && !known {
        return None;
    }
    let bold =
        bold || flags & FORCE_BOLD != 0 || has(&["bold", "black", "heavy", "semibold", "demi"]);
    let italic = italic || flags & ITALIC != 0 || has(&["italic", "oblique", "ital"]);
    let face = match (fixed, serif) {
        (true, _) => [FIXED, FIXED_BOLD, FIXED_ITALIC, FIXED_BOLD_ITALIC],
        (false, true) => [SERIF_REGULAR, SERIF_BOLD, SERIF_ITALIC, SERIF_BOLD_ITALIC],
        (false, false) => [SANS, SANS_BOLD, SANS_ITALIC, SANS_BOLD_ITALIC],
    };
    Some(face[usize::from(bold) + 2 * usize::from(italic)])
}

const SANS: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSans.cff");
const SANS_BOLD: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSansBold.cff");
const SANS_ITALIC: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSansItalic.cff");
const SANS_BOLD_ITALIC: &[u8] =
    include_bytes!("../../assets/standard-fonts/FoxitSansBoldItalic.cff");
const SERIF_REGULAR: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSerif.cff");
const SERIF_BOLD: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSerifBold.cff");
const SERIF_ITALIC: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSerifItalic.cff");
const SERIF_BOLD_ITALIC: &[u8] =
    include_bytes!("../../assets/standard-fonts/FoxitSerifBoldItalic.cff");
const FIXED: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitFixed.cff");
const FIXED_BOLD: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitFixedBold.cff");
const FIXED_ITALIC: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitFixedItalic.cff");
const FIXED_BOLD_ITALIC: &[u8] =
    include_bytes!("../../assets/standard-fonts/FoxitFixedBoldItalic.cff");
const SYMBOL: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitSymbol.cff");
const DINGBATS: &[u8] = include_bytes!("../../assets/standard-fonts/FoxitDingbats.cff");

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::raw::ps::cff::CffFontRef;

    #[test]
    fn every_face_parses_as_a_cff_program() {
        for face in [
            SANS,
            SANS_BOLD,
            SANS_ITALIC,
            SANS_BOLD_ITALIC,
            SERIF_REGULAR,
            SERIF_BOLD,
            SERIF_ITALIC,
            SERIF_BOLD_ITALIC,
            FIXED,
            FIXED_BOLD,
            FIXED_ITALIC,
            FIXED_BOLD_ITALIC,
            SYMBOL,
            DINGBATS,
        ] {
            let font = CffFontRef::new_cff(face, 0, None).expect("a stand-in face parses");
            assert!(font.num_glyphs() > 1);
        }
    }

    #[test]
    fn the_standard_14_names_pick_their_class() {
        assert_eq!(stand_in("Helvetica", None, false, false), Some(SANS));
        assert_eq!(
            stand_in("Helvetica-BoldOblique", None, false, false),
            Some(SANS_BOLD_ITALIC)
        );
        assert_eq!(
            stand_in("Times-Roman", None, false, false),
            Some(SERIF_REGULAR)
        );
        assert_eq!(
            stand_in("Times-Italic", None, false, false),
            Some(SERIF_ITALIC)
        );
        assert_eq!(
            stand_in("Courier-Bold", None, false, false),
            Some(FIXED_BOLD)
        );
        assert_eq!(stand_in("Symbol", None, false, false), Some(SYMBOL));
        assert_eq!(stand_in("ZapfDingbats", None, false, false), Some(DINGBATS));
    }

    #[test]
    fn system_fonts_pick_their_class_by_name_flags_and_declared_style() {
        assert_eq!(
            stand_in("ABCDEF+Arial,Bold", Some(32), false, false),
            Some(SANS_BOLD)
        );
        assert_eq!(
            stand_in("TimesNewRomanPSMT", Some(34), false, false),
            Some(SERIF_REGULAR)
        );
        assert_eq!(
            stand_in("CourierNewPSMT", Some(35), false, false),
            Some(FIXED)
        );
        assert_eq!(
            stand_in("SomeFace", Some(34 | 64), false, false),
            Some(SERIF_ITALIC)
        );
        assert_eq!(stand_in("SomeFace", Some(32), true, false), Some(SANS_BOLD));
        assert_eq!(
            stand_in("NotoSans-Regular", Some(2 | 32), false, false),
            Some(SANS)
        );
    }

    #[test]
    fn an_unknown_symbolic_font_has_no_stand_in() {
        assert_eq!(stand_in("Wingdings-Regular", Some(4), false, false), None);
        // A known text family flagged symbolic, as generators often do, still gets one.
        assert_eq!(stand_in("Arial", Some(4), false, false), Some(SANS));
    }
}
